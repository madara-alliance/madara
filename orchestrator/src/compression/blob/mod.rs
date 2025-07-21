mod da;
mod state_update;

use crate::compression::blob::da::{build_da_word_pre_v0_13_3, build_da_word_v0_13_3_and_above};
use crate::compression::blob::state_update::{
    add_contract_diffs_to_blob_data, add_declared_classes_to_blob_data, convert_to_biguint,
    process_addresses_with_storage_diffs, process_addresses_without_storage_diffs, BlobContractDiff,
};
use crate::core::config::StarknetVersion;
use crate::error::job::JobError;
use crate::types::constant::BLOB_LEN;
use crate::worker::event_handler::jobs::da::DAJobHandler;
use num_bigint::BigUint;
use starknet_core::types::{Felt, StateUpdate};
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
    // Create a state_diff copy
    let state_diff = state_update.state_diff;

    // Create a vector to hold the blob data
    // Initialize with placeholder for total contract count (will update later)
    let mut blob_data: Vec<Felt> = vec![Felt::ZERO];

    // Create a vector to hold the contract diffs
    let mut contract_diffs: Vec<BlobContractDiff> = Vec::new();

    // Create maps for an easier lookup
    let mut deployed_contracts: HashMap<Felt, Felt> =
        state_diff.deployed_contracts.into_iter().map(|item| (item.address, item.class_hash)).collect();
    let mut replaced_classes: HashMap<Felt, Felt> =
        state_diff.replaced_classes.into_iter().map(|item| (item.contract_address, item.class_hash)).collect();
    let mut nonces: HashMap<Felt, Felt> =
        state_diff.nonces.into_iter().map(|item| (item.contract_address, item.nonce)).collect();

    // Keep track of processed addresses
    let mut processed_addresses = HashSet::new();

    let len_storage_diffs = state_diff.storage_diffs.len();

    // Process each storage diff
    process_addresses_with_storage_diffs(
        state_diff.storage_diffs,
        version,
        &mut contract_diffs,
        &mut processed_addresses,
        &mut deployed_contracts,
        &mut replaced_classes,
        &mut nonces,
    )?;

    // Process leftover nonces (contracts that have nonce updates but no storage updates)
    let leftover_count = process_addresses_without_storage_diffs(
        &processed_addresses,
        &mut deployed_contracts,
        &mut replaced_classes,
        &mut nonces,
        version,
        &mut contract_diffs,
    )?;

    // Update the first element with the total number of contracts (original storage diffs and leftover addresses)
    let total_contracts = len_storage_diffs + leftover_count;
    blob_data[0] = Felt::from(total_contracts);

    // Sorting all the contract diffs by address
    contract_diffs.sort_by_key(|contract_diff| contract_diff.address);

    // Add all contract diffs to blob data
    add_contract_diffs_to_blob_data(contract_diffs, &mut blob_data)?;

    tracing::debug!("Total contract updates in blob: {}", total_contracts);

    // Add declared classes count and data
    add_declared_classes_to_blob_data(state_diff.declared_classes, &mut blob_data);

    tracing::debug!("Created blob data with {} elements", blob_data.len());

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
pub fn da_word(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
    version: StarknetVersion,
) -> Result<Felt, JobError> {
    let da_word: BigUint = if version >= StarknetVersion::V0_13_3 {
        build_da_word_v0_13_3_and_above(class_flag, nonce_change, num_changes)?
    } else {
        build_da_word_pre_v0_13_3(class_flag, nonce_change, num_changes)?
    };

    Ok(Felt::from(da_word))
}

/// Converts a vector of felts into blob data (vec of big uint)
/// Returns a vector of blobs
/// A single blob has a fixed size of [BLOB_LEN]
pub fn convert_felt_vec_to_blob_data(elements: &[Felt]) -> Result<Vec<Vec<BigUint>>, JobError> {
    let blob_data = convert_to_biguint(elements);
    let num_blobs = blob_data.len().div_ceil(BLOB_LEN);
    let mut transformed_data = Vec::new();
    for i in 0..num_blobs {
        transformed_data
            .push(DAJobHandler::fft_transformation(blob_data[BLOB_LEN * i..(BLOB_LEN * (i + 1))].to_vec())?);
    }
    Ok(transformed_data)
}
