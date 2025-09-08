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

pub mod fact_info;
pub mod fact_node;
pub mod fact_topology;

pub mod conversion;

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

/// fetch_blob_data_for_batch - Fetches blob data for a specific batch
/// Fetching the blob data (stored in remote storage during Batching) for a particular batch
///
/// # Arguments
/// * `index` - The index of the batch
/// * `config` - The configuration object
/// * `blob_data_paths` - A slice of blob data directory paths
///
/// # Returns
/// * `Result<Vec<Vec<u8>>, JobError>` - A result containing a vector of blob data or an error.
///
pub async fn fetch_blob_data_for_batch(
    index: usize,
    config: Arc<Config>,
    blob_data_paths: &[String],
) -> Result<Vec<Vec<u8>>, JobError> {
    // TODO: Check if we can optimize the memory usage here
    tracing::debug!("Fetching blob data for batch index {}", index);

    let storage_client = config.storage();

    // Get the directory path for this batch
    let dir_path = blob_data_paths.get(index).ok_or_else(|| {
        tracing::error!("Blob data path not found for index {}", index);
        JobError::Other(OtherError(eyre!("Blob data path not found for index {}", index)))
    })?;

    // Get the blob files for this batch
    let mut files = config.storage().list_files_in_dir(dir_path).await.map_err(|e| {
        tracing::error!("Failed to list files in path {}: {}", dir_path, e);
        JobError::Other(OtherError(eyre!("Failed to list files in path {}: {}", dir_path, e)))
    })?;
    // Sort the files by name (which is the blob index)
    files.sort();

    tracing::debug!("Retrieving blob data from {} files in directory: {}", files.len(), dir_path);

    let mut blobs = Vec::new();

    for file in files {
        let blob_data = storage_client.get_data(&file).await.map_err(|e| {
            tracing::error!("Failed to retrieve blob data from path {}: {}", file, e);
            JobError::Other(OtherError(eyre!("Failed to retrieve blob data from path {}: {}", file, e)))
        })?;
        blobs.push(blob_data.to_vec());
    }

    tracing::debug!("Successfully retrieved blob data for batch index {}", index);
    Ok(blobs)
}

/// fetch_blob_data_for_block - Fetches blob data for a specific block index.
/// Fetching the blob data (stored in remote storage during DA job) for a particular block
///
/// # Arguments
/// * `block_index` - The index of the block.
/// * `config` - The configuration object.
/// * `blob_data_paths` - A slice of blob data paths.
///
/// # Returns
/// * `Result<Vec<Vec<u8>>, JobError>` - A result containing a vector of blob data or an error.
///
pub async fn fetch_blob_data_for_block(
    block_index: usize,
    config: Arc<Config>,
    blob_data_paths: &[String],
) -> Result<Vec<Vec<u8>>, JobError> {
    tracing::debug!("Fetching blob data for block index {}", block_index);

    let storage_client = config.storage();

    // Get the path for this block
    let path = blob_data_paths.get(block_index).ok_or_else(|| {
        tracing::error!("Blob data path not found for index {}", block_index);
        JobError::Other(OtherError(eyre!("Blob data path not found for index {}", block_index)))
    })?;

    tracing::debug!("Retrieving blob data from path: {}", path);
    let blob_data = storage_client.get_data(path).await.map_err(|e| {
        tracing::error!("Failed to retrieve blob data from path {}: {}", path, e);
        JobError::Other(OtherError(eyre!("Failed to retrieve blob data from path {}: {}", path, e)))
    })?;

    tracing::debug!("Successfully retrieved blob data for block index {}", block_index);
    Ok(vec![blob_data.to_vec()])
}

/// fetch_snos_for_block - Fetches the SNOS output for a specific block index.
/// Retrieves the SNOS output (stored in remote storage during SNOS job) for a particular block.
///
/// # Arguments
/// * `internal_id` - The internal ID of the job.
/// * `index` - The index of the block.
/// * `config` - The configuration object.
/// * `snos_output_paths` - A slice of SNOS output paths.
///
/// # Returns
/// * `Result<StarknetOsOutput, JobError>` - A result containing the SNOS output or an error.
///
pub async fn fetch_snos_for_block(
    internal_id: String,
    index: usize,
    config: Arc<Config>,
    snos_output_paths: &[String],
) -> Result<Vec<Felt>, JobError> {
    tracing::debug!(job_id = %internal_id, "Fetching SNOS output for block index {}", index);

    let storage_client = config.storage();

    let snos_path = snos_output_paths.get(index).ok_or_else(|| {
        tracing::error!(job_id = %internal_id, "SNOS path not found for index {}", index);
        JobError::Other(OtherError(eyre!("Failed to get the SNOS path for job ID {}", internal_id)))
    })?;

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

pub async fn fetch_program_output_for_block(
    block_index: usize,
    config: Arc<Config>,
    program_output_paths: &[String],
) -> Result<Vec<[u8; 32]>, JobError> {
    tracing::debug!("Fetching program output for block index {}", block_index);

    let storage_client = config.storage();

    // Get the path for this block
    let path = program_output_paths.get(block_index).ok_or_else(|| {
        tracing::error!("Program output path not found for index {}", block_index);
        JobError::Other(OtherError(eyre!("Program output path not found for index {}", block_index)))
    })?;

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
