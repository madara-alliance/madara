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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `[u8; 32]` with explicit high (index 0) and low (index 31) bytes
    /// so we can distinguish big-endian and little-endian Felt decoding.
    fn bytes32(high: u8, low: u8) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0] = high;
        b[31] = low;
        b
    }

    #[test]
    fn empty_children_emits_only_count() {
        let out = build_bootloader_output(&[]);
        // Spec: with no children the bootloader still emits the count (= 0)
        // and nothing else. Pins that we never produce a stray header.
        assert_eq!(out, vec![Felt::from(0u64)]);
    }

    #[test]
    fn single_child_layout() {
        let child = vec![bytes32(0, 0x11), bytes32(0, 0x22), bytes32(0, 0x33)];
        let out = build_bootloader_output(std::slice::from_ref(&child));

        // Layout: [num_children, output_size, program_hash, ..child_output]
        // with output_size = child_len + 2 (for the size and hash fields).
        assert_eq!(out.len(), 1 + 2 + child.len());
        assert_eq!(out[0], Felt::from(1u64), "num_children");
        assert_eq!(out[1], Felt::from(5u64), "output_size = child_len + 2");
        assert_eq!(out[2], PROGRAM_HASHES.os, "program_hash = PROGRAM_HASHES.os");
        assert_eq!(out[3], Felt::from_bytes_be(&child[0]));
        assert_eq!(out[4], Felt::from_bytes_be(&child[1]));
        assert_eq!(out[5], Felt::from_bytes_be(&child[2]));
    }

    #[test]
    fn multiple_children_header_layout() {
        let child_a = vec![bytes32(0, 0xA1), bytes32(0, 0xA2)]; // len 2
        let child_b = vec![bytes32(0, 0xB1), bytes32(0, 0xB2), bytes32(0, 0xB3), bytes32(0, 0xB4)]; // len 4

        let out = build_bootloader_output(&[child_a.clone(), child_b.clone()]);

        // 1 (count) + (2 header + 2 body) + (2 header + 4 body) = 11 felts
        assert_eq!(out.len(), 11);
        assert_eq!(out[0], Felt::from(2u64), "num_children");

        // child_a header + body
        assert_eq!(out[1], Felt::from(4u64), "child_a output_size = 2 + 2");
        assert_eq!(out[2], PROGRAM_HASHES.os);
        assert_eq!(out[3], Felt::from_bytes_be(&child_a[0]));
        assert_eq!(out[4], Felt::from_bytes_be(&child_a[1]));

        // child_b header + body
        assert_eq!(out[5], Felt::from(6u64), "child_b output_size = 4 + 2");
        assert_eq!(out[6], PROGRAM_HASHES.os);
        assert_eq!(out[7], Felt::from_bytes_be(&child_b[0]));
        assert_eq!(out[8], Felt::from_bytes_be(&child_b[1]));
        assert_eq!(out[9], Felt::from_bytes_be(&child_b[2]));
        assert_eq!(out[10], Felt::from_bytes_be(&child_b[3]));
    }

    #[test]
    fn child_felt_encoding_big_endian() {
        // With high byte = 0x01 and low byte = 0xFF the BE and LE Felt values
        // differ, so a regression from `from_bytes_be` to `from_bytes_le`
        // would make this assertion fail loudly.
        let bytes = bytes32(0x01, 0xFF);
        let out = build_bootloader_output(&[vec![bytes]]);

        let expected_be = Felt::from_bytes_be(&bytes);
        let expected_le = Felt::from_bytes_le(&bytes);
        assert_ne!(expected_be, expected_le, "sentinel: BE and LE must differ for this input");

        assert_eq!(out[3], expected_be, "child felt must be big-endian decoded");
        assert_ne!(out[3], expected_le, "child felt must not be little-endian decoded");
    }
}
