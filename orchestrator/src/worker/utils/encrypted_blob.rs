//! DA segment extraction and KZG proof verification utilities.
//!
//! The DA segment is provided separately from the CairoPIE (e.g., from the prover).
//! This module provides utilities to:
//! 1. Load the DA segment
//! 2. Apply FFT transformation
//! 3. Verify KZG proof matches the CairoPIE's program output

use crate::types::constant::BLOB_LEN;
use crate::worker::event_handler::jobs::da::DAJobHandler;
use crate::worker::utils::biguint_vec_to_u8_vec;
use cairo_vm::Felt252;
use num_bigint::BigUint;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaSegmentError {
    #[error("DA segment is empty")]
    EmptyDaSegment,

    #[error("FFT transformation failed: {0}")]
    FftError(String),

    #[error("Invalid hex string: {0}")]
    InvalidHex(String),
}

/// Convert DA segment felts to blob bytes.
///
/// **L2 Usage**: This function is used for L2 settlements where the DA segment
/// is provided by the prover (Atlantic) as encrypted/compressed state diff data.
/// The prover generates this segment by encrypting the state diff with symmetric
/// keys derived from public keys.
///
/// For L3 settlements where state diffs are generated locally, use
/// [`convert_felt_vec_to_blob_data`] instead.
///
/// # Process
/// 1. Pads to BLOB_LEN (4096) boundary
/// 2. Applies FFT transformation for KZG commitment
/// 3. Converts to blob bytes (131072 bytes per blob = 4096 * 32)
///
/// # Arguments
/// * `da_segment` - The DA segment as a vector of Felt252 (from prover)
///
/// # Returns
/// * `Vec<Vec<u8>>` - A vector of blobs, each blob is 131072 bytes
///
/// [`convert_felt_vec_to_blob_data`]: crate::compression::blob::convert_felt_vec_to_blob_data
pub fn da_segment_to_blobs(da_segment: Vec<Felt252>) -> Result<Vec<Vec<u8>>, DaSegmentError> {
    if da_segment.is_empty() {
        return Err(DaSegmentError::EmptyDaSegment);
    }

    // Calculate number of blobs needed
    let num_blobs = da_segment.len().div_ceil(BLOB_LEN);

    // Convert to BigUint and pad to full blob length
    let da_biguint: Vec<BigUint> = da_segment.iter().map(|f| f.to_biguint()).collect();
    let mut padded = da_biguint;
    padded.resize(num_blobs * BLOB_LEN, BigUint::from(0u8));

    // Process each blob
    let mut blobs = Vec::with_capacity(num_blobs);
    for i in 0..num_blobs {
        let chunk = padded[i * BLOB_LEN..(i + 1) * BLOB_LEN].to_vec();

        // Apply FFT transformation
        let transformed =
            DAJobHandler::fft_transformation(chunk).map_err(|e| DaSegmentError::FftError(e.to_string()))?;

        // Convert to bytes
        let blob_bytes = biguint_vec_to_u8_vec(&transformed);
        blobs.push(blob_bytes);
    }

    Ok(blobs)
}

/// Parse DA segment from JSON array of hex strings.
///
/// # Arguments
/// * `json_str` - JSON string containing array of hex strings (e.g., `["0x1", "0x2", ...]`)
///
/// # Returns
/// * `Vec<Felt252>` - The parsed DA segment
pub fn parse_da_segment_json(json_str: &str) -> Result<Vec<Felt252>, DaSegmentError> {
    let hex_strings: Vec<String> =
        serde_json::from_str(json_str).map_err(|e| DaSegmentError::InvalidHex(e.to_string()))?;

    hex_strings
        .iter()
        .map(|hex| Felt252::from_hex(hex).map_err(|e| DaSegmentError::InvalidHex(e.to_string())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairo_vm::vm::runners::cairo_pie::CairoPie;
    use orchestrator_ethereum_settlement_client::EthereumSettlementClient;
    use orchestrator_utils::test_utils::setup_test_data;
    use rstest::rstest;

    // Test artifacts source:
    //
    // Paradex testnet aggregator batches (L2 with encrypted DA):
    // - index_1_aggregator_14_1.zip + da_blob_index_1.json: Batch 1, blocks 490000-490001
    // - index_2_aggregator_14_1.zip + da_blob_index_2.json: Batch 2, blocks 490002-490003
    //
    // Legacy test artifacts (from aggregator-poc repo):
    // - test_aggregator.zip and testing_aggregator.json are from commit aef88ee1b2f686c5b50cf83621bc24516a93f8f4
    // - Repository: https://github.com/Mohiiit/aggregator-poc.git

    /// Verifies that the DA segment from the prover can be converted to blobs
    /// and that the KZG proof matches the program output from the CairoPIE.
    ///
    /// This test validates the full L2 DA flow:
    /// 1. Extract program output from aggregator CairoPIE
    /// 2. Load DA segment (pre-FFT encrypted state diff from prover)
    /// 3. Apply FFT transformation and convert to blobs
    /// 4. Verify KZG commitment matches (y_0 in program output = KZG eval of blob)
    #[rstest]
    #[case("index_1_aggregator_14_1.zip", "da_blob_index_1.json")]
    #[case("index_2_aggregator_14_1.zip", "da_blob_index_2.json")]
    #[tokio::test]
    async fn test_da_segment_kzg_verification(#[case] cairo_pie_file: &str, #[case] da_segment_file: &str) {
        use crate::worker::utils::fact_info::get_program_output;

        dotenvy::from_filename_override("../.env.test").expect("Failed to load .env.test file");

        // Download test artifacts from remote repository
        let data_dir = setup_test_data(vec![(cairo_pie_file, false), (da_segment_file, false)])
            .await
            .expect("Failed to download test artifacts");

        // Load CairoPIE and extract program output
        let cairo_pie =
            CairoPie::read_zip_file(&data_dir.path().join(cairo_pie_file)).expect("Failed to load CairoPIE");
        let program_output_felts = get_program_output(&cairo_pie, true).expect("Failed to get program output");
        let program_output: Vec<[u8; 32]> = program_output_felts.iter().map(|f| f.to_bytes_be()).collect();

        // Load DA segment and convert to blobs (applies FFT transformation)
        let da_json =
            std::fs::read_to_string(data_dir.path().join(da_segment_file)).expect("Failed to read DA segment");
        let da_segment = parse_da_segment_json(&da_json).expect("Failed to parse DA segment");
        let blobs = da_segment_to_blobs(da_segment).expect("Failed to convert DA segment to blobs");

        // Verify KZG proof matches - build_input_bytes internally validates y_0 values
        EthereumSettlementClient::build_input_bytes(program_output, blobs)
            .await
            .expect("KZG proof verification failed - y_0 mismatch between DA segment and program output");
    }
}
