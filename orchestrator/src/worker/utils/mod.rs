use num_bigint::BigUint;
use std::fmt::Write;
use std::io::{BufRead, Cursor};
use std::str::FromStr;
use std::sync::Arc;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use alloy::primitives::U256;
use color_eyre::eyre::eyre;
pub(crate) use orchestrator_ethereum_settlement_client::conversion::hex_string_to_u8_vec;
// use starknet_os::io::output::StarknetOsOutput;
use starknet_core::types::Felt;

pub mod conversion;
pub mod encrypted_blob;
pub mod fact_info;
pub mod fact_node;
pub mod fact_topology;

/// biguint_vec_to_u8_vec - Converts a vector of BigUint numbers to a vector of u8 bytes.
///
/// # Arguments
/// * `nums` - A slice of BigUint numbers.
///
/// # Returns
/// * `Vec<u8>` - A vector of u8 bytes representing the BigUint numbers.
pub fn biguint_vec_to_u8_vec(nums: &[BigUint]) -> Vec<u8> {
    nums.iter().flat_map(biguint_to_32_bytes).collect()
}

/// biguint_to_32_bytes - Converts a BigUint number to a fixed-size array of 32 bytes.
///
/// # Arguments
/// * `num` - A reference to a BigUint number.
///
/// # Returns
/// * `[u8; 32]` - A fixed-size array of 32 bytes representing the BigUint number.
///
pub fn biguint_to_32_bytes(num: &BigUint) -> [u8; 32] {
    let bytes = num.to_bytes_be();
    let mut result = [0u8; 32];

    if bytes.len() > 32 {
        // If we have more than 32 bytes, take only the last 32
        result.copy_from_slice(&bytes[bytes.len() - 32..]);
    } else {
        // If we have 32 or fewer bytes, pad with zeros at the beginning
        result[32 - bytes.len()..].copy_from_slice(&bytes);
    }

    result
}

/// fetch_blob_data - Fetches blob data for the state update job.
/// Fetching the blob data (stored in remote storage during DA job) for a particular block/batch.
///
/// # Arguments
/// * `config` - The configuration object.
/// * `blob_data_path` - Optional path to the blob data file.
///
/// # Returns
/// * `Result<Vec<Vec<u8>>, JobError>` - A result containing a vector of blob data or an error.
///
pub async fn fetch_blob_data(config: Arc<Config>, blob_data_path: &Option<String>) -> Result<Vec<Vec<u8>>, JobError> {
    let path = blob_data_path.as_ref().ok_or_else(|| {
        tracing::error!("Blob data path not provided");
        JobError::Other(OtherError(eyre!("Blob data path not provided")))
    })?;

    tracing::debug!("Fetching blob data from path: {}", path);

    let storage_client = config.storage();

    tracing::debug!("Retrieving blob data from path: {}", path);
    let blob_data = storage_client.get_data(path).await.map_err(|e| {
        tracing::error!("Failed to retrieve blob data from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to retrieve blob data from path {}: {}", path, e)))
    })?;

    tracing::debug!("Successfully retrieved blob data from path: {}", path);
    Ok(vec![blob_data.to_vec()])
}

/// fetch_da_segment - Fetches and parses the DA segment for the state update job.
/// The DA segment contains the encrypted/compressed state diff from the prover.
/// This data is used to generate blob data for KZG proof verification.
///
/// # Arguments
/// * `config` - The configuration object
/// * `da_segment_path` - Optional path to the DA segment file
///
/// # Returns
/// * `Result<Vec<Vec<u8>>, JobError>` - Blob data ready for KZG proof
///
pub async fn fetch_da_segment(config: Arc<Config>, da_segment_path: &Option<String>) -> Result<Vec<Vec<u8>>, JobError> {
    use crate::worker::utils::encrypted_blob::{da_segment_to_blobs, parse_da_segment_json};

    let path = da_segment_path.as_ref().ok_or_else(|| {
        tracing::error!("DA segment path not provided");
        JobError::Other(OtherError(eyre!("DA segment path not provided")))
    })?;

    tracing::debug!("Fetching DA segment from path: {}", path);

    let storage_client = config.storage();

    tracing::debug!("Retrieving DA segment from path: {}", path);
    let da_segment_bytes = storage_client.get_data(path).await.map_err(|e| {
        tracing::error!("Failed to retrieve DA segment from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to retrieve DA segment from path {}: {}", path, e)))
    })?;

    // Parse JSON to get the DA segment felts
    let json_str = String::from_utf8(da_segment_bytes.to_vec()).map_err(|e| {
        tracing::error!("Failed to parse DA segment as UTF-8 from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to parse DA segment as UTF-8 from path {}: {}", path, e)))
    })?;

    let da_segment = parse_da_segment_json(&json_str).map_err(|e| {
        tracing::error!("Failed to parse DA segment JSON from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to parse DA segment JSON from path {}: {}", path, e)))
    })?;

    // Convert DA segment to blob bytes (applies FFT transformation)
    let blobs = da_segment_to_blobs(da_segment).map_err(|e| {
        tracing::error!("Failed to convert DA segment to blobs: {}", e);
        JobError::Other(OtherError(eyre!("Failed to convert DA segment to blobs: {}", e)))
    })?;

    tracing::debug!("Successfully converted DA segment to {} blobs from path: {}", blobs.len(), path);
    Ok(blobs)
}

/// fetch_snos_output - Fetches the SNOS output for the state update job.
/// Retrieves the SNOS output (stored in remote storage during SNOS job).
///
/// # Arguments
/// * `internal_id` - The internal ID of the job.
/// * `config` - The configuration object.
/// * `snos_output_path` - Optional path to the SNOS output file.
///
/// # Returns
/// * `Result<Vec<Felt>, JobError>` - A result containing the SNOS output or an error.
///
pub async fn fetch_snos_output(
    internal_id: u64,
    config: Arc<Config>,
    snos_output_path: &Option<String>,
) -> Result<Vec<Felt>, JobError> {
    let snos_path = snos_output_path.as_ref().ok_or_else(|| {
        tracing::error!(job_id = %internal_id, "SNOS output path not provided");
        JobError::Other(OtherError(eyre!("SNOS output path not provided for job ID {}", internal_id)))
    })?;

    tracing::debug!(job_id = %internal_id, "Fetching SNOS output from path: {}", snos_path);

    let storage_client = config.storage();

    tracing::debug!(job_id = %internal_id, "Retrieving SNOS output from path: {}", snos_path);
    let snos_output_bytes = storage_client.get_data(snos_path).await.map_err(|e| {
        tracing::error!(job_id = %internal_id, "Failed to retrieve SNOS data from path {}: {}", snos_path, e);
        JobError::Other(OtherError(eyre!("Failed to retrieve SNOS data from path {}: {}", snos_path, e)))
    })?;

    tracing::debug!(job_id = %internal_id, "Deserializing SNOS output from path: {}", snos_path);
    serde_json::from_slice(snos_output_bytes.iter().as_slice()).map_err(|e| {
        tracing::error!(
            job_id = %internal_id,
            "Failed to deserialize SNOS output from path {}: {}",
            snos_path, e
        );
        JobError::Other(OtherError(eyre!("Failed to deserialize SNOS output from path {}: {}", snos_path, e)))
    })
}

/// fetch_program_output - Fetches the program output for the state update job.
///
/// # Arguments
/// * `config` - The configuration object.
/// * `program_output_path` - Optional path to the program output file.
///
/// # Returns
/// * `Result<Vec<[u8; 32]>, JobError>` - A result containing the program output or an error.
///
pub async fn fetch_program_output(
    config: Arc<Config>,
    program_output_path: &Option<String>,
) -> Result<Vec<[u8; 32]>, JobError> {
    let path = program_output_path.as_ref().ok_or_else(|| {
        tracing::error!("Program output path not provided");
        JobError::Other(OtherError(eyre!("Program output path not provided")))
    })?;

    tracing::debug!("Fetching program output from path: {}", path);

    let storage_client = config.storage();

    tracing::debug!("Retrieving program output from path: {}", path);
    let program_output = storage_client.get_data(path).await.map_err(|e| {
        tracing::error!("Failed to retrieve program output from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to retrieve program output from path {}: {}", path, e)))
    })?;

    tracing::debug!("Deserializing program output from path: {}", path);
    bincode::deserialize(&program_output).map_err(|e| {
        tracing::error!("Failed to deserialize program output from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to deserialize program output from path {}: {}", path, e)))
    })
}

// Util Functions
// ===============

pub fn bytes_to_vec_u8(bytes: &[u8]) -> color_eyre::Result<Vec<[u8; 32]>> {
    let cursor = Cursor::new(bytes);
    let reader = std::io::BufReader::new(cursor);

    let mut program_output: Vec<[u8; 32]> = Vec::new();

    for line in reader.lines() {
        let line = line.expect("can't read line");
        let trimmed = line.trim();
        assert!(!trimmed.is_empty());

        let result = U256::from_str(trimmed)?;
        let res_vec = result.to_be_bytes_vec();
        let hex = to_padded_hex(res_vec.as_slice());
        let vec_hex = hex_string_to_u8_vec(&hex)
            .map_err(|e| eyre!(format!("Failed converting hex string to Vec<u8>, {:?}", e)))?;
        program_output
            .push(vec_hex.try_into().map_err(|e| eyre!(format!("Failed to convert Vec<u8> to [u8; 32] : {:?}", e)))?);
    }

    Ok(program_output)
}

fn to_padded_hex(slice: &[u8]) -> String {
    assert!(slice.len() <= 32, "Slice length must not exceed 32");
    let hex = slice.iter().fold(String::new(), |mut output, byte| {
        // 0: pads with zeros
        // 2: specifies the minimum width (2 characters)
        // x: formats the number as lowercase hexadecimal
        // writes a byte value as a two-digit hexadecimal number (padded with a leading zero if necessary)
        // to the specified output.
        let _ = write!(output, "{byte:02x}");
        output
    });
    format!("{:0<64}", hex)
}
