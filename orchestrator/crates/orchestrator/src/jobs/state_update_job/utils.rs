use std::fmt::Write;
use std::io::{BufRead, Cursor};
use std::str::FromStr;
use std::sync::Arc;

use alloy::primitives::U256;
use color_eyre::eyre::eyre;
use num_bigint::BigUint;

use crate::config::Config;
use crate::constants::{BLOB_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME};

/// Fetching the blob data (stored in remote storage during DA job) for a particular block
pub async fn fetch_blob_data_for_block(block_number: u64, config: Arc<Config>) -> color_eyre::Result<Vec<Vec<u8>>> {
    let storage_client = config.storage();
    let key = block_number.to_string() + "/" + BLOB_DATA_FILE_NAME;
    let blob_data = storage_client.get_data(&key).await?;
    Ok(vec![blob_data.to_vec()])
}

/// Fetching the blob data (stored in remote storage during DA job) for a particular block
pub async fn fetch_program_data_for_block(block_number: u64, config: Arc<Config>) -> color_eyre::Result<Vec<[u8; 32]>> {
    let storage_client = config.storage();
    let key = block_number.to_string() + "/" + PROGRAM_OUTPUT_FILE_NAME;
    let blob_data = storage_client.get_data(&key).await?;
    let transformed_blob_vec_u8 = bytes_to_vec_u8(blob_data.as_ref())?;
    Ok(transformed_blob_vec_u8)
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

pub fn biguint_vec_to_u8_vec(nums: &[BigUint]) -> Vec<u8> {
    let mut result: Vec<u8> = Vec::new();

    for num in nums {
        result.extend_from_slice(biguint_to_32_bytes(num).as_slice());
    }

    result
}

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
