pub mod error;

use cairo_vm::types::builtin_name::BuiltinName;
use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::types::relocatable::MaybeRelocatable;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use cairo_vm::Felt252;
use error::AggregatorRunnerError;
use starknet_os::hint_processor::aggregator_hint_processor::{AggregatorInput, DataAvailability};
use starknet_os::runner::run_aggregator;
// Re-export Felt for callers that don't depend on starknet-types-core directly
pub use starknet_types_core::felt::Felt as AggregatorFelt;
use starknet_types_core::felt::Felt;

/// Input configuration for running the local aggregator.
pub struct AggregatorRunnerInput {
    /// The child CairoPIEs to aggregate (one per SNOS batch).
    pub child_cairo_pies: Vec<CairoPie>,
    /// Cairo VM layout to use.
    pub layout: LayoutName,
    /// Whether to include full output (typically false for L2).
    pub full_output: bool,
    /// Enable debug logging in the aggregator.
    pub debug_mode: bool,
    /// The chain ID as a Felt.
    pub chain_id: Felt,
    /// The fee token address.
    pub fee_token_address: Felt,
    /// DA public keys for state diff encryption.
    pub da_public_keys: Option<Vec<Felt>>,
}

/// Output from the local aggregator run.
pub struct AggregatorRunnerOutput {
    /// The aggregator CairoPIE (to be submitted as an applicative job).
    pub aggregator_cairo_pie: CairoPie,
    /// The aggregator program output.
    pub aggregator_output: Vec<Felt>,
    /// The DA segment, serialized as JSON bytes.
    /// This is read from the temp file where the aggregator writes it.
    pub da_segment: Vec<u8>,
}

/// Run the aggregator locally to produce an aggregator CairoPIE and DA segment.
///
/// This is used in the SHARP L2 flow where the orchestrator must create the
/// aggregator PIE itself (unlike Atlantic which does it remotely).
///
/// # Process
/// 1. Extract program output from each child CairoPIE
/// 2. Build the bootloader output vector
/// 3. Run the aggregator program via `starknet_os::runner::run_aggregator`
/// 4. Read the DA segment from the temp file where the aggregator wrote it
/// 5. Validate the output CairoPIE
pub fn run_local_aggregator(input: AggregatorRunnerInput) -> Result<AggregatorRunnerOutput, AggregatorRunnerError> {
    if input.child_cairo_pies.is_empty() {
        return Err(AggregatorRunnerError::NoChildPies);
    }

    tracing::info!(num_children = input.child_cairo_pies.len(), "Building aggregator input from child CairoPIEs");

    // 1. Build bootloader output from child CairoPIEs
    let bootloader_output = build_bootloader_output(&input.child_cairo_pies)?;

    // 2. Create temp file for DA segment output.
    //    The aggregator writes the DA segment to this path during execution.
    //    TODO: Upstream PR to starknet_os to support in-memory DA segment output,
    //    avoiding this temp file round-trip.
    let da_temp_file = tempfile::NamedTempFile::new()?;
    let da_path = da_temp_file.path().to_path_buf();

    // 3. Build AggregatorInput and run the aggregator
    let aggregator_input = AggregatorInput {
        bootloader_output: Some(bootloader_output),
        full_output: input.full_output,
        debug_mode: input.debug_mode,
        chain_id: input.chain_id,
        da: DataAvailability::Blob(da_path.clone()),
        public_keys: input.da_public_keys,
        fee_token_address: input.fee_token_address,
    };

    tracing::info!("Running aggregator program");
    let output = run_aggregator(input.layout, aggregator_input)
        .map_err(|e| AggregatorRunnerError::AggregatorExecution(e.to_string()))?;

    // 4. Read DA segment from temp file
    let da_segment = std::fs::read(&da_path).map_err(|e| {
        AggregatorRunnerError::DaSegmentRead(format!("Failed to read DA segment from {:?}: {}", da_path, e))
    })?;

    tracing::info!(da_segment_len = da_segment.len(), "DA segment read from temp file");

    // 5. Validate the output CairoPIE
    output.cairo_pie.run_validity_checks().map_err(|e| AggregatorRunnerError::PieValidation(format!("{:?}", e)))?;

    tracing::info!("Aggregator CairoPIE validated successfully");

    Ok(AggregatorRunnerOutput {
        aggregator_cairo_pie: output.cairo_pie,
        aggregator_output: output.aggregator_output,
        da_segment,
    })
}

/// Build the bootloader_output Vec<Felt> from child CairoPIEs.
///
/// Format: `[num_children, output_size_1, program_hash, ...child_1_output..., ...]`
///
/// For each child:
/// - `output_size` = child output length + 2 (for the size and hash fields)
/// - `program_hash` = hash of the SNOS program
/// - followed by the child's program output felts
fn build_bootloader_output(child_pies: &[CairoPie]) -> Result<Vec<Felt>, AggregatorRunnerError> {
    // Use the pre-computed OS program hash from the embedded program_hash.json.
    // This is a LazyLock that loads once on first access - no redundant computation.
    let os_program_hash = apollo_starknet_os_program::PROGRAM_HASHES.os;

    let mut output = Vec::new();

    // Number of children
    output.push(Felt::from(child_pies.len()));

    for (idx, pie) in child_pies.iter().enumerate() {
        tracing::debug!(child_index = idx, "Extracting program output from child CairoPIE");

        let child_output = get_program_output_from_pie(pie)?;
        let output_size = child_output.len() + 2; // +2 for output_size and program_hash

        output.push(Felt::from(output_size));
        output.push(os_program_hash);

        // Convert child output (Felt252 from cairo-vm) to Felt (from starknet-types-core)
        for felt252 in child_output {
            output.push(Felt::from_bytes_be(&felt252.to_bytes_be()));
        }
    }

    tracing::info!(total_felts = output.len(), "Built bootloader output");
    Ok(output)
}

/// Extract program output from a CairoPIE's output builtin segment.
///
/// This is the same logic as `orchestrator::worker::utils::fact_info::get_program_output`
/// with `is_aggregator=false` (child PIEs are not aggregator PIEs).
fn get_program_output_from_pie(cairo_pie: &CairoPie) -> Result<Vec<Felt252>, AggregatorRunnerError> {
    let segment_info = cairo_pie
        .metadata
        .builtin_segments
        .get(&BuiltinName::output)
        .ok_or(AggregatorRunnerError::OutputBuiltinNoSegmentInfo)?;

    let mut output = vec![Felt252::from(0); segment_info.size];
    let mut insertion_count = 0;
    let cairo_program_memory = &cairo_pie.memory.0;

    for ((index, offset), value) in cairo_program_memory.iter() {
        if *index == segment_info.index as usize {
            match value {
                MaybeRelocatable::Int(felt) => {
                    output[*offset] = *felt;
                    insertion_count += 1;
                }
                MaybeRelocatable::RelocatableValue(_) => {
                    return Err(AggregatorRunnerError::OutputSegmentUnexpectedRelocatable(*offset));
                }
            }
        }
    }

    if insertion_count != segment_info.size {
        return Err(AggregatorRunnerError::IncompleteOutputSegment {
            expected: segment_info.size,
            found: insertion_count,
        });
    }

    Ok(output)
}
