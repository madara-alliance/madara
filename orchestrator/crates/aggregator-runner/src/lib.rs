pub mod error;

use cairo_vm::types::layout_name::LayoutName;
use error::AggregatorRunnerError;
use starknet_os::hint_processor::aggregator_hint_processor::{AggregatorInput, DataAvailability};
use starknet_os::runner::run_aggregator;
// Re-export Felt for callers that don't depend on starknet-types-core directly
pub use starknet_types_core::felt::Felt as AggregatorFelt;
use starknet_types_core::felt::Felt;

// Re-export the pre-computed program hashes so the orchestrator can consume them
// without pulling `apollo_starknet_os_program` directly.
pub use apollo_starknet_os_program::PROGRAM_HASHES;

/// Input configuration for running the local aggregator.
pub struct AggregatorRunnerInput {
    /// Pre-stored program outputs for each child SNOS batch.
    /// Each element is a `Vec<[u8; 32]>` (bincode-deserialized from S3).
    /// This avoids loading full CairoPIEs (20-40MB each) into memory.
    pub child_program_outputs: Vec<Vec<[u8; 32]>>,
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
    pub aggregator_cairo_pie: cairo_vm::vm::runners::cairo_pie::CairoPie,
    /// The aggregator program output.
    pub aggregator_output: Vec<Felt>,
    /// The DA segment, serialized as JSON bytes.
    /// Read from the temp file where the aggregator writes it.
    pub da_segment: Vec<u8>,
}

/// Run the aggregator locally to produce an aggregator CairoPIE and DA segment.
///
/// This is used in the SHARP L2 flow where the orchestrator must create the
/// aggregator PIE itself (unlike Atlantic which does it remotely).
///
/// # Process
/// 1. Build the bootloader output vector from pre-stored program outputs
/// 2. Run the aggregator program via `starknet_os::runner::run_aggregator`
/// 3. Read the DA segment from the temp file where the aggregator wrote it
/// 4. Validate the output CairoPIE
pub fn run_local_aggregator(input: AggregatorRunnerInput) -> Result<AggregatorRunnerOutput, AggregatorRunnerError> {
    if input.child_program_outputs.is_empty() {
        return Err(AggregatorRunnerError::NoChildOutputs);
    }

    tracing::info!(
        num_children = input.child_program_outputs.len(),
        "Building aggregator input from child program outputs"
    );

    // 1. Build bootloader output from pre-stored program outputs
    let bootloader_output = build_bootloader_output(&input.child_program_outputs);

    // 2. Create temp file for DA segment output.
    //    The aggregator writes the DA segment to this path during execution.
    //    TODO(@prakhar, 2026-04-16): Upstream PR to starknet_os to support in-memory DA segment output.
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

/// Build the bootloader_output Vec<Felt> from pre-stored program outputs.
///
/// Format: `[num_children, output_size_1, program_hash, ...child_1_output..., ...]`
///
/// For each child:
/// - `output_size` = child output length + 2 (for the size and hash fields)
/// - `program_hash` = pre-computed SNOS program hash from PROGRAM_HASHES
/// - followed by the child's program output felts
fn build_bootloader_output(child_outputs: &[Vec<[u8; 32]>]) -> Vec<Felt> {
    // Use the pre-computed OS program hash from the embedded program_hash.json.
    // This is a LazyLock that loads once on first access.
    let os_program_hash = PROGRAM_HASHES.os;

    let mut output = Vec::new();

    // Number of children
    output.push(Felt::from(child_outputs.len()));

    for (idx, child_output) in child_outputs.iter().enumerate() {
        tracing::debug!(child_index = idx, output_len = child_output.len(), "Adding child program output");

        let output_size = child_output.len() + 2; // +2 for output_size and program_hash
        output.push(Felt::from(output_size));
        output.push(os_program_hash);

        // Convert [u8; 32] → Felt
        for bytes in child_output {
            output.push(Felt::from_bytes_be(bytes));
        }
    }

    tracing::info!(total_felts = output.len(), "Built bootloader output");
    output
}
