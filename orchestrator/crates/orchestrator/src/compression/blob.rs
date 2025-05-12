use crate::compression::constants::{BLOB_LEN, BLS_MODULUS, GENERATOR, ONE};
use crate::core::config::StarknetVersion;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use num_bigint::BigUint;
use num_traits::{Num, Zero};
use rayon::prelude::*;
use starknet_core::types::{ContractStorageDiffItem, DeclaredClassItem, Felt, StateUpdate};
use std::collections::{HashMap, HashSet};

/// Converts a StateUpdate to a vector of Felt values for blob creation
///
/// # Arguments
/// * `state_update` - The StateUpdate to convert
/// * `version` - Version of the starknet to be used
///
/// # Returns
/// A vector of Felt values representing the state update in a format suitable for blob creation
pub async fn state_update_to_blob_data(
    state_update: StateUpdate,
    version: StarknetVersion,
) -> Result<Vec<Felt>, JobError> {
    let mut state_diff = state_update.state_diff;

    // Create a vector to hold the blob data
    // Initialize with placeholder for total contract count (will update later)
    let mut blob_data: Vec<Felt> = vec![Felt::ZERO];

    // Create maps for an easier lookup
    let mut deployed_contracts: HashMap<Felt, Felt> =
        state_diff.deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();
    let mut replaced_classes: HashMap<Felt, Felt> =
        state_diff.replaced_classes.into_iter().map(|item| (item.contract_address, item.class_hash)).collect();
    let mut nonces: HashMap<Felt, Felt> =
        state_diff.nonces.into_iter().map(|item| (item.contract_address, item.nonce)).collect();

    // Keep track of processed addresses
    let mut processed_addresses = HashSet::new();

    // Sort storage diffs by address for deterministic output
    state_diff.storage_diffs.sort_by_key(|diff| diff.address);

    // Process each storage diff
    for ContractStorageDiffItem { address, mut storage_entries } in state_diff.storage_diffs.clone().into_iter() {
        // Mark this address as processed
        processed_addresses.insert(address);

        // Get class hash from deployed_contracts or replaced_classes
        let mut class_hash = None;

        // First, try deployed contracts
        if let Some(hash) = deployed_contracts.remove(&address) {
            // println!("Found deployed contract at address {}: class_hash={}", address, hash);
            class_hash = Some(hash);
        }
        // Then try replaced classes
        else if let Some(hash) = replaced_classes.remove(&address) {
            // println!("Found replaced class at address {}: class_hash={}", address, hash);
            class_hash = Some(hash);
        }

        // Get nonce if it exists and remove from the map since we're processing this address
        let nonce = nonces.remove(&address);

        // Create the DA word - class_flag is true if class_hash is Some
        let da_word = da_word(class_hash.is_some(), nonce, storage_entries.len() as u64, version)?;

        // Add address and DA word to blob data
        blob_data.push(address);
        blob_data.push(da_word);

        // If there's a class hash, add it to blob data
        if let Some(hash) = class_hash {
            blob_data.push(hash);
            // println!("Adding class hash {} for address {}", hash, address);
        }

        // Sort storage entries by key for deterministic output
        storage_entries.sort_by_key(|entry| entry.key);

        // Add storage entries to blob data
        for entry in storage_entries {
            blob_data.push(entry.key);
            blob_data.push(entry.value);
        }
    }

    // Process leftover nonces (contracts that have nonce updates but no storage updates)
    let mut leftover_addresses = Vec::new();

    // Check for any addresses with deployed contracts that weren't processed
    for (address, class_hash) in deployed_contracts.iter() {
        if !processed_addresses.contains(address) {
            leftover_addresses.push((*address, Some(*class_hash), nonces.remove(address)));
            // println!("Found leftover deployed contract: address={}, class_hash={}", address, class_hash);
        }
    }

    // Check for any addresses with replaced classes that weren't processed
    for (address, class_hash) in replaced_classes.iter() {
        if !processed_addresses.contains(address) {
            leftover_addresses.push((*address, Some(*class_hash), nonces.remove(address)));
            // println!("Found leftover replaced class: address={}, class_hash={}", address, class_hash);
        }
    }

    // Check for any addresses with only nonce updates that weren't processed
    for (address, nonce) in nonces.iter() {
        if !processed_addresses.contains(address) {
            leftover_addresses.push((*address, None, Some(*nonce)));
            // println!("Found leftover nonce: address={}, nonce={}", address, nonce);
        }
    }

    // Sort leftover addresses for deterministic output
    leftover_addresses.sort_by_key(|(address, _, _)| *address);

    // println!(
    //     "Processing {} leftover addresses with nonce or class updates but no storage updates",
    //     leftover_addresses.len()
    // );

    // Process each leftover address
    for (address, class_hash, nonce) in leftover_addresses.clone() {
        // Create DA word with zero storage entries
        let da_word = da_word(class_hash.is_some(), nonce, 0, version)?;

        // println!(
        //     "Processing leftover address {}: class_hash={:?}, nonce={:?}",
        //     address,
        //     class_hash.map(|h| h.to_string()),
        //     nonce.map(|n| n.to_string())
        // );

        // Add address and DA word to blob data
        blob_data.push(address);
        blob_data.push(da_word);

        // If there's a class hash, add it to blob data
        if let Some(hash) = class_hash {
            blob_data.push(hash);
            // println!("Adding class hash {} for leftover address {}", hash, address);
        }
    }

    // Update the first element with the total number of contracts (original storage diffs and leftover addresses)
    let total_contracts = state_diff.storage_diffs.len() + leftover_addresses.len();
    blob_data[0] = Felt::from(total_contracts);
    // println!("Total contract updates in blob: {}", total_contracts);

    // Add declared classes count
    blob_data.push(Felt::from(state_diff.declared_classes.len()));

    // Sort declared classes by class_hash for deterministic output
    state_diff.declared_classes.sort_by_key(|class| class.class_hash);

    // Process each declared class
    for DeclaredClassItem { class_hash, compiled_class_hash } in state_diff.declared_classes.into_iter() {
        blob_data.push(class_hash);
        blob_data.push(compiled_class_hash);
    }

    // println!("Created blob data with {} elements", blob_data.len());

    Ok(blob_data)
}

/// Creates a DA word with information about a contract
///
/// # Arguments
/// * `class_flag` - Indicates if a new class hash is present
/// * `nonce_change` 0 Optional nonce value as a Felt
/// * `num_change` - Number of storage updates
/// * `version` - Version string to determine the encoding format
///
/// # Returns
/// A `Felt` representing the encoded DA word
fn da_word(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
    version: StarknetVersion,
) -> Result<Felt, JobError> {
    // Parse version to determine format
    let is_new_version = version >= StarknetVersion::V0_13_3;
    let mut binary_string = String::new();
    if is_new_version {
        // v0.13.3+ format:
        // - new_nonce (64 bits)
        // - n_updates (8 or 64 bits depending on size)
        // - n_updates_len (1 bit)
        // - class_flag (1 bit)

        // Add new_nonce (64 bits)
        if let Some(new_nonce) = nonce_change {
            let bytes: [u8; 32] = new_nonce.to_bytes_be();
            let big_uint_value = BigUint::from_bytes_be(&bytes);
            let nonce_binary = format!("{:b}", big_uint_value);
            binary_string += &format!("{:0>64}", nonce_binary);
        } else {
            // If nonce unchanged, use 64 zeros
            binary_string += &"0".repeat(64);
        }

        // Determine if we need 8 or 64 bits for num_changes
        let needs_large_updates = num_changes >= 256;

        if needs_large_updates {
            // Use 64 bits for updates
            let updates_binary = format!("{:b}", num_changes);
            binary_string += &format!("{:0>64}", updates_binary);
            // Add remaining padding to reach 254 bits
            binary_string = format!("{:0>254}", binary_string);
            // Add n_updates_len (1) and class_flag
            binary_string.push('0'); // n_updates_len = 1 for large updates
        } else {
            // Use 8 bits for updates
            let updates_binary = format!("{:b}", num_changes);
            binary_string += &format!("{:0>8}", updates_binary);
            // Add remaining padding to reach 254 bits
            binary_string = format!("{:0>254}", binary_string);
            // Add n_updates_len (0) and class_flag
            binary_string.push('1'); // n_updates_len = 0 for small updates
        }

        // Add class_flag as LSB
        binary_string.push(if class_flag { '1' } else { '0' });
    } else {
        // Old format (pre-0.13.3)
        // padding of 127 bits
        binary_string = "0".repeat(127);

        // class flag of one bit
        if class_flag {
            binary_string += "1"
        } else {
            binary_string += "0"
        }

        // checking for nonce here
        if let Some(new_nonce) = nonce_change {
            let bytes: [u8; 32] = new_nonce.to_bytes_be();
            let binary_string_local = format!("{:b}", BigUint::from_bytes_be(&bytes));
            let padded_binary_string = format!("{:0>64}", binary_string_local);
            binary_string += &padded_binary_string;
        } else {
            let binary_string_local = "0".repeat(64);
            binary_string += &binary_string_local;
        }

        let binary_representation = format!("{:b}", num_changes);
        let padded_binary_string = format!("{:0>64}", binary_representation);
        binary_string += &padded_binary_string;
    }

    // Convert binary string to decimal string
    let decimal_string = BigUint::from_str_radix(binary_string.as_str(), 2)
        .map_err(|e| {
            JobError::Other(OtherError(color_eyre::eyre::eyre!("Failed to convert binary string to BigUint: {e}")))
        })?
        .to_str_radix(10);

    Felt::from_dec_str(&decimal_string).map_err(|e| {
        JobError::Other(OtherError(color_eyre::eyre::eyre!("Failed to convert decimal string to FieldElement: {}", e)))
    })
}

pub fn convert_to_biguint(elements: &[Felt]) -> Vec<BigUint> {
    let input_len = elements.len();
    if input_len == 0 {
        return Vec::new(); // Return empty vector for empty input
    }

    // Calculate the required output size: ceil(input_len / 4096.0) * 4096
    // Integer division trick: (input_len + 4095) / 4096 gives the ceiling division result
    let num_blocks = (input_len + 4095) / 4096;
    let output_len = num_blocks * 4096;

    // Initialize the vector with the calculated size, filled with zeros
    let mut biguint_vec = vec![BigUint::zero(); output_len];

    // Iterate over the input elements and place them in the output vector
    for (i, element) in elements.iter().enumerate() {
        // Remove .take(4096)
        // Convert Felt to [u8; 32]
        let bytes: [u8; 32] = element.to_bytes_be();

        // Convert [u8; 32] to BigUint
        let biguint = BigUint::from_bytes_be(&bytes);

        // Place the converted value at the correct index
        // This automatically leaves remaining spots as zeros
        biguint_vec[i] = biguint;
    }

    biguint_vec
}

/// Perform Number Theoretic Transform (NTT) on a vector of BigUint values
///
/// # Arguments
/// * `arr` - Vector of BigUint values
/// * `xs` - Evaluation points
/// * `p` - Modulus (typically BLS_MODULUS)
///
/// # Returns
/// Transformed vector of BigUint values
pub fn ntt(arr: Vec<BigUint>, xs: Vec<BigUint>, p: &BigUint) -> Vec<BigUint> {
    // Use Rayon for parallel processing
    (0..arr.len())
        .into_par_iter()
        .map(|i| {
            let mut result = BigUint::zero();
            let mut xi_pow_j = ONE.clone(); // Initialize to xs[i]**0

            for j in 0..arr.len() {
                let term = (&arr[j] * &xi_pow_j) % p;
                result = (result + term) % p;
                xi_pow_j = (&xi_pow_j * &xs[i]) % p; // Update power for next iteration
            }

            result
        })
        .collect()
}

/// Generate evaluation points for NTT/IFFT operations
///
/// # Arguments
/// * `size` - Size of the data (default: BLOB_LEN)
///
/// # Returns
/// A tuple of (evaluation points, modulus)
pub fn generate_evaluation_points(size: Option<usize>) -> (Vec<BigUint>, BigUint) {
    let blob_len = size.unwrap_or(BLOB_LEN);

    // Generate evaluation points
    let xs: Vec<BigUint> = (0..blob_len)
        .map(|i| {
            let bin = format!("{:012b}", i);
            let bin_rev = bin.chars().rev().collect::<String>();
            GENERATOR.modpow(&BigUint::from_str_radix(&bin_rev, 2).unwrap(), &BLS_MODULUS)
        })
        .collect();

    (xs, BLS_MODULUS.clone())
}

/// Process a vector of BigUint values for blob data transformation using NTT
///
/// # Arguments
/// * `data` - Vector of BigUint values to process
/// * `size` - Optional size parameter to override BLOB_LEN (default: BLOB_LEN)
///
/// # Returns
/// Transformed vector for blob data
pub fn process_for_blob(data: Vec<BigUint>, size: Option<usize>) -> Vec<BigUint> {
    let blob_len = size.unwrap_or(BLOB_LEN);

    // Get evaluation points
    let (xs, p) = generate_evaluation_points(Some(blob_len));

    // Ensure data is of correct length
    let mut data_padded = data;
    if data_padded.len() < blob_len {
        // Pad with zeros if needed
        data_padded.resize(blob_len, BigUint::zero());
    } else if data_padded.len() > blob_len {
        // Truncate if too long
        data_padded.truncate(blob_len);
    }

    // Perform NTT transformation
    let transformed_data = ntt(data_padded, xs, &p);

    transformed_data
}

/// Create a blob from BigUint data
///
/// # Arguments
/// * `data` - Vector of BigUint values
///
/// # Returns
/// Blob data as a Vec<u8>
pub fn create_blob_from_data(data: Vec<BigUint>) -> Vec<u8> {
    // Convert BigUint to bytes
    let mut blob_data = Vec::new();
    for num in data {
        let bytes = num.to_bytes_be();
        blob_data.extend_from_slice(&bytes);
    }
    blob_data
}

pub fn convert_felt_vec_to_blob_data(elements: &[Felt]) -> String {
    create_blob_from_data(process_for_blob(convert_to_biguint(elements), None))
        .iter()
        .map(|x| x.to_string())
        .collect()
    // transformed_data.into_iter().map(|x| Felt::from(x)).collect()
}
