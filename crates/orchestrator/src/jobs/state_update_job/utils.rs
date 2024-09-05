use std::sync::Arc;

use crate::config::Config;
use crate::constants::BLOB_DATA_FILE_NAME;
use color_eyre::eyre::eyre;

/// Fetching the blob data (stored in remote storage during DA job) for a particular block
pub async fn fetch_blob_data_for_block(block_number: u64, config: Arc<Config>) -> color_eyre::Result<Vec<Vec<u8>>> {
    let storage_client = config.storage();
    let key = block_number.to_string() + "/" + BLOB_DATA_FILE_NAME;
    let blob_data = storage_client.get_data(&key).await?;
    let blob_vec_data: Vec<Vec<u8>> =
        bincode::deserialize(&blob_data).expect("Not able to convert Vec<u8> to Vec<Vec<u8>> during deserialization.");
    Ok(blob_vec_data)
}

// Util Functions
// ===============

/// Util function to convert hex string data into Vec<u8>
pub fn hex_string_to_u8_vec(hex_str: &str) -> color_eyre::Result<Vec<u8>> {
    // Remove any spaces or non-hex characters from the input string
    let cleaned_str: String = hex_str.chars().filter(|c| c.is_ascii_hexdigit()).collect();

    // Convert the cleaned hex string to a Vec<u8>
    let mut result = Vec::new();
    for chunk in cleaned_str.as_bytes().chunks(2) {
        if let Ok(byte_val) = u8::from_str_radix(std::str::from_utf8(chunk)?, 16) {
            result.push(byte_val);
        } else {
            return Err(eyre!("Error parsing hex string: {}", cleaned_str));
        }
    }

    Ok(result)
}
