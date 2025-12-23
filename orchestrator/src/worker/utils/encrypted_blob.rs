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
/// The DA segment is pre-FFT data. This function:
/// 1. Pads to BLOB_LEN (4096) boundary
/// 2. Applies FFT transformation
/// 3. Converts to blob bytes
///
/// # Arguments
/// * `da_segment` - The DA segment as a vector of Felt252
///
/// # Returns
/// * `Vec<Vec<u8>>` - A vector of blobs, each blob is 131072 bytes (4096 * 32)
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
    use c_kzg::{Blob, Bytes32};
    use cairo_vm::vm::runners::cairo_pie::CairoPie;
    use orchestrator_ethereum_settlement_client::KZG_SETTINGS;
    use std::path::PathBuf;

    fn get_test_cairo_pie_path() -> PathBuf {
        [env!("CARGO_MANIFEST_DIR"), "src", "tests", "artifacts", "test_aggregator.zip"].iter().collect()
    }

    fn get_test_da_segment_path() -> PathBuf {
        [env!("CARGO_MANIFEST_DIR"), "src", "tests", "artifacts", "testing_aggregator.json"].iter().collect()
    }

    #[test]
    fn test_da_segment_kzg_verification() {
        use crate::worker::utils::fact_info::get_program_output;

        // 1. Load CairoPIE
        let pie_path = get_test_cairo_pie_path();
        let cairo_pie = CairoPie::read_zip_file(&pie_path).expect("Failed to load CairoPIE");

        // 2. Get program output
        let program_output = get_program_output(&cairo_pie, true).expect("Failed to get program output");

        println!("\n=== Program Output ===");
        println!("Length: {} felts", program_output.len());
        println!("[10] x_0: {:?}", program_output.get(10));
        println!("[11] n_blobs: {:?}", program_output.get(11));
        println!("[14] y_0_low: {:?}", program_output.get(14));
        println!("[15] y_0_high: {:?}", program_output.get(15));

        // 3. Load DA segment
        let da_path = get_test_da_segment_path();
        let da_json = std::fs::read_to_string(&da_path).expect("Failed to read DA segment file");
        let da_segment = parse_da_segment_json(&da_json).expect("Failed to parse DA segment");

        println!("\n=== DA Segment ===");
        println!("Length: {} felts", da_segment.len());

        // 4. Convert DA segment to blob bytes
        let blobs = da_segment_to_blobs(da_segment).expect("Failed to convert DA segment to blobs");

        println!("Generated {} blob(s)", blobs.len());
        assert_eq!(blobs.len(), 1, "Expected 1 blob");

        // 5. Get x_0 and expected y_0 from program output
        let x_0_felt = &program_output[10];
        let x_0 = Bytes32::new(x_0_felt.to_bytes_be());

        let y_0_low = &program_output[14];
        let y_0_high = &program_output[15];

        // Combine y_0: high || low (big-endian 256-bit)
        let mut expected_y_0 = [0u8; 32];
        let y_0_low_bytes = y_0_low.to_bytes_be();
        let y_0_high_bytes = y_0_high.to_bytes_be();
        expected_y_0[0..16].copy_from_slice(&y_0_high_bytes[16..32]);
        expected_y_0[16..32].copy_from_slice(&y_0_low_bytes[16..32]);

        println!("\n=== KZG Verification ===");
        println!("x_0: {}", hex::encode(x_0.as_slice()));
        println!("Expected y_0: {}", hex::encode(&expected_y_0));

        // 6. Compute KZG proof from blob
        let blob_bytes: [u8; 131072] = blobs[0].clone().try_into().expect("Invalid blob size");
        let blob = Blob::new(blob_bytes);

        let commitment = KZG_SETTINGS.blob_to_kzg_commitment(&blob).expect("Failed to compute commitment");
        println!("Commitment: {:?}", commitment);

        let (_proof, computed_y_0) = KZG_SETTINGS.compute_kzg_proof(&blob, &x_0).expect("Failed to compute proof");
        println!("Computed y_0: {}", hex::encode(computed_y_0.as_slice()));

        // 7. Verify y_0 matches
        if computed_y_0.as_slice() == expected_y_0 {
            println!("\n✅ SUCCESS! KZG proof verification passed!");
        } else {
            println!("\n❌ FAILED! y_0 values do not match.");
            println!("The DA segment and CairoPIE may be from different runs.");
        }

        assert_eq!(
            computed_y_0.as_slice(),
            expected_y_0,
            "KZG proof verification failed: y_0 mismatch"
        );
    }
}
