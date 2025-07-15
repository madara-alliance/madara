mod compare;

use crate::compression::blob::convert_to_biguint;
use crate::compression::stateless::decompress;
use crate::core::StorageClient;
use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::tests::jobs::snos_job::SNOS_PATHFINDER_RPC_URL_ENV;
use crate::tests::utils::read_file_to_string;
use crate::types::batch::{ClassDeclaration, ContractUpdate, DataJson, StorageUpdate};
use crate::worker::event_handler::triggers::batching::BatchingTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use crate::worker::utils::hex_string_to_u8_vec;
use alloy::hex;
use color_eyre::Result;
use majin_blob_core::blob;
use num_bigint::BigUint;
use num_traits::{ToPrimitive, Zero};
use rstest::*;
use starknet_core::types::Felt;
use std::collections::HashMap;
use tracing::log::info;
use tracing::{error, warn};
use url::Url;

#[rstest]
#[case("src/tests/artifacts/8373665/blobs/")]
#[tokio::test]
async fn test_assign_batch_to_block_new_batch(#[case] blob_dir: String) -> Result<()> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            warn!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return Ok(());
        }
    };

    let services = TestConfigBuilder::new()
        .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url)))
        .configure_storage_client(ConfigType::Actual)
        .configure_database(ConfigType::Actual)
        .build()
        .await;

    let result = BatchingTrigger.run_worker(services.config.clone()).await;

    assert!(result.is_ok());

    let generated_blobs =
        get_blobs_from_s3_paths(vec!["blob/batch/1/1.txt", "blob/batch/1/2.txt"], services.config.storage()).await?;

    let real_blobs = get_blobs_from_files(vec![&format!("{blob_dir}1.txt"), &format!("{blob_dir}2.txt")])?;

    assert_eq!(generated_blobs[0], real_blobs[0]);
    assert_eq!(generated_blobs[1], real_blobs[1]);

    Ok(())
}

async fn get_blobs_from_s3_paths(s3_paths: Vec<&str>, storage: &dyn StorageClient) -> Result<Vec<String>> {
    let mut blob: Vec<String> = Vec::new();
    for path in s3_paths {
        blob.push(hex::encode(storage.get_data(path).await?));
    }
    Ok(blob)
}

fn get_blobs_from_files(file_paths: Vec<&str>) -> Result<Vec<String>> {
    let mut blob: Vec<String> = Vec::new();
    for path in file_paths {
        blob.push(read_file_to_string(path)?);
    }
    Ok(blob)
}

fn get_data_json_from_vec_biguints(data: &[BigUint], version: &str) -> Result<DataJson> {
    let vec_felts = convert_biguints_to_felts(data)?;
    let decompressed = convert_to_biguint(&decompress(&vec_felts)?);
    Ok(parse_state_diffs(&decompressed, version))
}

fn get_felt_vec(hex_str: &str) -> Result<Vec<BigUint>> {
    let blob_data = hex_string_to_u8_vec(hex_str)?;
    let recovered_data = bytes_to_biguints(&blob_data);
    Ok(blob::recover(recovered_data.clone()))
}

/// Convert bytes to a vector of BigUint
pub fn bytes_to_biguints(bytes: &[u8]) -> Vec<BigUint> {
    let mut result = Vec::new();
    for chunk in bytes.chunks(32) {
        let mut padded = [0u8; 32];
        let len = std::cmp::min(chunk.len(), 32);
        padded[32 - len..].copy_from_slice(&chunk[..len]);
        let num = BigUint::from_bytes_be(&padded);
        result.push(num);
    }

    info!("Converted {} bytes to {} BigUint values", bytes.len(), result.len());
    result
}

/// Converts a vector of BigUint values to a vector of Felt values
///
/// # Arguments
/// * `biguints` - Vector of BigUint values to convert
///
/// # Returns
/// A Result containing a vector of Felt values or an error
pub fn convert_biguints_to_felts(biguints: &[BigUint]) -> Result<Vec<Felt>> {
    biguints
        .iter()
        .map(|b| {
            let bytes = b.to_bytes_be();
            // Handle empty bytes case
            if bytes.is_empty() {
                return Ok(Felt::ZERO);
            }

            // Create a fixed size array for the bytes
            let mut field_bytes = [0u8; 32];

            // Copy bytes, padding with zeros if needed
            if bytes.len() <= 32 {
                let start_idx = 32 - bytes.len();
                field_bytes[start_idx..].copy_from_slice(&bytes);
            } else {
                // Truncate if bigger than 32 bytes
                field_bytes.copy_from_slice(&bytes[bytes.len() - 32..]);
            }

            // Convert to Felt
            Ok(Felt::from_bytes_be(&field_bytes))
        })
        .collect()
}

/// Updated parse_state_diffs function with version support
pub fn parse_state_diffs(data: &[BigUint], version: &str) -> DataJson {
    if data.is_empty() {
        error!("Error: Empty data array");
        return DataJson {
            state_update_size: 0,
            state_update: Vec::new(),
            class_declaration_size: 0,
            class_declaration: Vec::new(),
        };
    }

    // Parse version string to determine which format to use
    let is_new_version = {
        let version_parts: Vec<u32> = version.split('.').filter_map(|s| s.parse().ok()).collect();

        if version_parts.len() >= 3 {
            // Compare with 0.13.3
            version_parts[0] > 0
                || (version_parts[0] == 0 && version_parts[1] > 13)
                || (version_parts[0] == 0 && version_parts[1] == 13 && version_parts[2] >= 3)
        } else {
            false // Default to old version if version string is invalid
        }
    };

    let mut updates = Vec::new();
    let mut i = 0;

    // 0th index has the number of contract updates
    let contract_updated_num = match data[i].to_usize() {
        Some(num) => num,
        None => {
            error!("Error: Could not parse number of contract updates");
            return DataJson {
                state_update_size: 0,
                state_update: Vec::new(),
                class_declaration_size: 0,
                class_declaration: Vec::new(),
            };
        }
    };
    i += 1;

    // 1st index should have a special address 0x1
    let special_address = &data[i];
    if special_address != &BigUint::from(1u32) {
        warn!("Warning: Expected special address 0x1 at index 1, found {}", special_address);
    }

    // Process contract updates
    for _ in 0..contract_updated_num {
        if i >= data.len() {
            break;
        }

        let address = data[i].clone();

        if address == BigUint::zero() {
            break;
        }
        i += 1;

        if i >= data.len() {
            break;
        }

        let info_word = &data[i];
        let (nonce, number_of_storage_updates, class_flag) = if is_new_version {
            let (new_nonce, storage_updates, class_flag) = extract_bits_v2(info_word);
            (new_nonce, storage_updates, class_flag)
        } else {
            let (class_flag, nonce, storage_updates) = extract_bits(info_word);
            (nonce, storage_updates, class_flag)
        };
        i += 1;

        let new_class_hash = if class_flag {
            if i >= data.len() {
                None
            } else {
                let hash = Some(data[i].clone());
                i += 1;
                hash
            }
        } else {
            None
        };

        let mut storage_updates = Vec::new();
        for _ in 0..number_of_storage_updates {
            if i + 1 >= data.len() {
                break;
            }

            let key = data[i].clone();
            i += 1;
            let value = data[i].clone();
            i += 1;

            if key == BigUint::zero() && value == BigUint::zero() {
                break;
            }

            storage_updates.push(StorageUpdate { key, value });
        }

        updates.push(ContractUpdate { address, nonce, number_of_storage_updates, new_class_hash, storage_updates });
    }

    // Process class declarations (remains the same for both versions)
    let declared_classes_len: usize = if i < data.len() { data[i].to_usize().unwrap_or(0) } else { 0 };
    i += 1;

    let mut class_declaration_updates = Vec::new();
    for _ in 0..declared_classes_len {
        if i >= data.len() {
            break;
        }

        let class_hash = data[i].clone();
        if class_hash == BigUint::zero() {
            break;
        }
        i += 1;

        if i >= data.len() {
            break;
        }

        let compiled_class_hash = data[i].clone();
        i += 1;

        class_declaration_updates.push(ClassDeclaration { class_hash, compiled_class_hash });
    }

    DataJson {
        state_update_size: (contract_updated_num).to_u64().unwrap_or(0),
        state_update: updates,
        class_declaration_size: declared_classes_len.to_u64().unwrap_or(0),
        class_declaration: class_declaration_updates,
    }
}

/// Function to extract bits based on version >= 0.13.3 format
/// # Arguments
/// * `info_word` - The `BigUint` to extract bits from.
/// # Returns
/// A tuple containing:
/// - new_nonce: u64 (0 if nonce unchanged)
/// - number_of_storage_updates: u64 (8 or 64 bits based on n_updates_len)
/// - class_flag: bool (indicates if class was replaced)
fn extract_bits_v2(info_word: &BigUint) -> (u64, u64, bool) {
    // converting the bigUint to binary
    let binary_string = format!("{:b}", info_word);
    // adding padding so that it can be of 256 length
    let bitstring = format!("{:0>256}", binary_string);
    if bitstring.len() != 256 {
        panic!("Input string must be 256 bits long");
    }

    // Reading from right to left (LSB is last bit):
    // - class_flag (1 bit)
    // - n_updates_len (1 bit)
    // - n_updates (8 or 64 bits depending on n_updates_len)
    // - new_nonce (64 bits)

    // Get class_flag (LSB)
    let class_flag = bitstring.chars().nth(255).unwrap() == '1';

    // Get n_updates_len (second bit from right)
    let n_updates_len = bitstring.chars().nth(254).unwrap() == '0';

    // Get number_of_storage_updates based on n_updates_len
    let number_of_storage_updates = if n_updates_len {
        // Use 64 bits for large number of updates
        // Reading 64 bits before the flags (bits 190-253)
        let updates_bits = &bitstring[190..254];
        u64::from_str_radix(updates_bits, 2).expect("Invalid binary string for large storage updates count")
    } else {
        // Use 8 bits for small number of updates
        // Reading 8 bits before the flags (bits 246-253)
        let updates_bits = &bitstring[246..254];
        u64::from_str_radix(updates_bits, 2).expect("Invalid binary string for small storage updates count")
    };

    // Get the new_nonce (64 bits)
    // Position depends on n_updates_len
    let new_nonce_bits = if n_updates_len {
        // If using 64 bits for updates, nonce is at bits 126-189
        &bitstring[126..190]
    } else {
        // If using 8 bits for updates, nonce is at bits 182-245
        &bitstring[182..246]
    };
    let new_nonce = u64::from_str_radix(new_nonce_bits, 2).expect("Invalid binary string for new nonce");

    // Note: new_nonce will be 0 if the nonce is unchanged
    (new_nonce, number_of_storage_updates, class_flag)
}

/// Function to extract class flag, nonce and state_diff length from a `BigUint`.
/// # Arguments
/// * `info_word` - The `BigUint` to extract bits from.
/// # Returns
/// A `bool` representing the class flag.
/// A `u64` representing the nonce.
/// Another`u64` representing the state_diff length
fn extract_bits(info_word: &BigUint) -> (bool, u64, u64) {
    // converting the bigUint to binary
    let binary_string = format!("{:b}", info_word);
    // adding padding so that it can be of 256 length
    let bitstring = format!("{:0>256}", binary_string);
    if bitstring.len() != 256 {
        panic!("Input string must be 256 bits long");
    }
    // getting the class flag, 127th bit is class flag (assuming 0 indexing)
    let class_flag_bit = &bitstring[127..128];
    // getting the nonce, nonce is of 64 bit from 128th bit to 191st bit
    let new_nonce_bits = &bitstring[128..192];
    // getting the state_diff_len, state_diff_len is of 64 bit from 192nd bit to 255th bit
    let num_changes_bits = &bitstring[192..256];

    // converting data to respective type
    let class_flag = class_flag_bit == "1";
    let new_nonce = u64::from_str_radix(new_nonce_bits, 2).expect("Invalid binary string for new nonce");
    let num_changes = u64::from_str_radix(num_changes_bits, 2).expect("Invalid binary string for num changes");

    (class_flag, new_nonce, num_changes)
}

/// Compares two ContractUpdate structures and outputs a detailed comparison
///
/// # Arguments
/// * `update1` - First ContractUpdate structure
/// * `update2` - Second ContractUpdate structure
///
/// # Returns
/// A string containing a detailed comparison of the two structures
fn compare_contract_updates(update1: &ContractUpdate, update2: &ContractUpdate) -> String {
    let mut result = String::new();
    let mut has_diff = false;

    // Compare nonces
    if update1.nonce != update2.nonce {
        result.push_str(&format!("- Nonce difference: {} vs {}\n", update1.nonce, update2.nonce));
        has_diff = true;
    }

    // Compare class hashes
    match (&update1.new_class_hash, &update2.new_class_hash) {
        (Some(hash1), Some(hash2)) => {
            if hash1 != hash2 {
                result.push_str(&format!("- Class Hash difference: {} vs {}\n", hash1, hash2));
                has_diff = true;
            }
        }
        (Some(hash1), None) => {
            result.push_str(&format!("- Class Hash in first file only: {}\n", hash1));
            has_diff = true;
        }
        (None, Some(hash2)) => {
            result.push_str(&format!("- Class Hash in second file only: {}\n", hash2));
            has_diff = true;
        }
        _ => {}
    }

    // Compare storage updates
    let mut storage_map1: HashMap<BigUint, BigUint> = HashMap::new();
    let mut storage_map2: HashMap<BigUint, BigUint> = HashMap::new();

    for storage in &update1.storage_updates {
        storage_map1.insert(storage.key.clone(), storage.value.clone());
    }

    for storage in &update2.storage_updates {
        storage_map2.insert(storage.key.clone(), storage.value.clone());
    }

    // Find keys in storage1 but not in storage2
    let mut only_in_storage1 = Vec::new();
    for (key, value) in &storage_map1 {
        if !storage_map2.contains_key(key) {
            only_in_storage1.push((key, value));
        }
    }

    // Find keys in storage2 but not in storage1
    let mut only_in_storage2 = Vec::new();
    for (key, value) in &storage_map2 {
        if !storage_map1.contains_key(key) {
            only_in_storage2.push((key, value));
        }
    }

    // Find keys with different values
    let mut different_values = Vec::new();
    for (key, value1) in &storage_map1 {
        if let Some(value2) = storage_map2.get(key) {
            if value1 != value2 {
                different_values.push((key, value1, value2));
            }
        }
    }

    if !only_in_storage1.is_empty() || !only_in_storage2.is_empty() || !different_values.is_empty() {
        has_diff = true;

        result.push_str(&format!("- Storage update differences:\n"));
        result.push_str(&format!("  - Storage keys only in first file: {}\n", only_in_storage1.len()));
        result.push_str(&format!("  - Storage keys only in second file: {}\n", only_in_storage2.len()));
        result.push_str(&format!("  - Storage keys with different values: {}\n", different_values.len()));

        // Show detailed storage differences
        if !only_in_storage1.is_empty() {
            result.push_str("  - Details of storage keys only in first file:\n");
            for (i, (key, value)) in only_in_storage1.iter().enumerate().take(5) {
                result.push_str(&format!("    - Key: {}, Value: {}\n", key, value));
                if i == 4 && only_in_storage1.len() > 5 {
                    result.push_str(&format!("    - ... and {} more\n", only_in_storage1.len() - 5));
                    break;
                }
            }
        }

        if !only_in_storage2.is_empty() {
            result.push_str("  - Details of storage keys only in second file:\n");
            for (i, (key, value)) in only_in_storage2.iter().enumerate().take(5) {
                result.push_str(&format!("    - Key: {}, Value: {}\n", key, value));
                if i == 4 && only_in_storage2.len() > 5 {
                    result.push_str(&format!("    - ... and {} more\n", only_in_storage2.len() - 5));
                    break;
                }
            }
        }

        if !different_values.is_empty() {
            result.push_str("  - Details of storage keys with different values:\n");
            for (i, (key, value1, value2)) in different_values.iter().enumerate().take(5) {
                result.push_str(&format!(
                    "    - Key: {}\n      - Value in first file: {}\n      - Value in second file: {}\n",
                    key, value1, value2
                ));
                if i == 4 && different_values.len() > 5 {
                    result.push_str(&format!("    - ... and {} more\n", different_values.len() - 5));
                    break;
                }
            }
        }
    }

    if has_diff {
        result
    } else {
        String::new()
    }
}
