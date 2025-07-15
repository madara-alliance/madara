use crate::core::config::StarknetVersion;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::constant::BLOB_LEN;
use crate::worker::event_handler::jobs::da::DAJobHandler;
use color_eyre::eyre::eyre;
use itertools::repeat_n;
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};
use starknet_core::types::{ContractStorageDiffItem, DeclaredClassItem, Felt, StateDiff, StateUpdate};
use std::collections::{HashMap, HashSet};

/// Prepares and sorts the state diff for deterministic output
fn prepare_state_diff(mut state_diff: StateDiff) -> StateDiff {
    // Sort storage diffs by address for deterministic output
    state_diff.storage_diffs.sort_by_key(|diff| diff.address);
    // For each storage diff, sort storage entries by key for deterministic output
    for diff in &mut state_diff.storage_diffs {
        diff.storage_entries.sort_by(|a, b| a.key.cmp(&b.key));
    }
    // Sort declared classes by class_hash for deterministic output
    state_diff.declared_classes.sort_by_key(|class| class.class_hash);
    state_diff
}

/// Processes storage diffs and updates blob_data, processed_addresses, and contract maps
fn process_storage_diffs(
    storage_diffs: Vec<ContractStorageDiffItem>,
    version: StarknetVersion,
    blob_data: &mut Vec<Felt>,
    processed_addresses: &mut HashSet<Felt>,
    deployed_contracts: &mut HashMap<Felt, Felt>,
    replaced_classes: &mut HashMap<Felt, Felt>,
    nonces: &mut HashMap<Felt, Felt>,
) -> Result<(), JobError> {
    for ContractStorageDiffItem { address, storage_entries } in storage_diffs.into_iter() {
        tracing::debug!("Processing storage diffs for address {}", address);
        // Mark this address as processed
        processed_addresses.insert(address);

        // Get class hash from deployed_contracts or replaced_classes
        let mut class_hash = None;

        // First, try deployed contracts
        if let Some(hash) = deployed_contracts.remove(&address) {
            tracing::debug!("Found deployed contract at address {} with class_hash {}", address, hash);
            class_hash = Some(hash);
        }
        // Then try replaced classes
        else if let Some(hash) = replaced_classes.remove(&address) {
            tracing::debug!("Found replaced class at address {} with class_hash {}", address, hash);
            class_hash = Some(hash);
        }

        // Get nonce if it exists and remove from the map since we're processing this address
        let nonce = nonces.remove(&address);

        // Create the DA word - class_flag is true if class_hash is Some (i.e., the class is replaced)
        let da_word = da_word(class_hash.is_some(), nonce, storage_entries.len() as u64, version)?;

        // Add address and DA word to blob data
        blob_data.push(address);
        blob_data.push(da_word);

        // If there's a class hash, add it to blob data
        if let Some(hash) = class_hash {
            blob_data.push(hash);
        }

        // Add storage entries to blob data
        for entry in storage_entries {
            blob_data.push(entry.key);
            blob_data.push(entry.value);
        }
    }
    Ok(())
}

/// Collects leftover addresses (nonce/class updates without storage) and returns a sorted vector
fn collect_leftover_addresses(
    processed_addresses: &HashSet<Felt>,
    deployed_contracts: &mut HashMap<Felt, Felt>,
    replaced_classes: &mut HashMap<Felt, Felt>,
    nonces: &mut HashMap<Felt, Felt>,
) -> Vec<(Felt, Option<Felt>, Option<Felt>)> {
    let mut leftover_addresses = Vec::new();
    leftover_addresses.extend(
        deployed_contracts
            .drain()
            .filter(|(address, _)| !processed_addresses.contains(address))
            .map(|(address, class_hash)| (address, Some(class_hash), nonces.remove(&address))),
    );
    leftover_addresses.extend(
        replaced_classes
            .drain()
            .filter(|(address, _)| !processed_addresses.contains(address))
            .map(|(address, class_hash)| (address, Some(class_hash), nonces.remove(&address))),
    );
    leftover_addresses.extend(
        nonces
            .drain()
            .filter(|(address, _)| !processed_addresses.contains(address))
            .map(|(address, nonce)| (address, None, Some(nonce))),
    );
    leftover_addresses.sort_by_key(|(address, _, _)| *address);
    leftover_addresses
}

/// Appends a single leftover address to blob_data
fn append_leftover_address_to_blob(
    address: Felt,
    class_hash: Option<Felt>,
    nonce: Option<Felt>,
    version: StarknetVersion,
    blob_data: &mut Vec<Felt>,
) -> Result<(), JobError> {
    let da_word = da_word(class_hash.is_some(), nonce, 0, version)?;
    tracing::debug!(
        "Processing leftover address {}: class_hash={:?}, nonce={:?}",
        address,
        class_hash.map(|h| h.to_string()),
        nonce.map(|n| n.to_string())
    );
    blob_data.push(address);
    blob_data.push(da_word);
    if let Some(hash) = class_hash {
        blob_data.push(hash);
    }
    Ok(())
}

/// Processes leftover addresses (nonce/class updates without storage) and updates blob_data
fn process_leftover_addresses(
    processed_addresses: &HashSet<Felt>,
    deployed_contracts: &mut HashMap<Felt, Felt>,
    replaced_classes: &mut HashMap<Felt, Felt>,
    nonces: &mut HashMap<Felt, Felt>,
    version: StarknetVersion,
    blob_data: &mut Vec<Felt>,
) -> Result<usize, JobError> {
    let leftover_addresses =
        collect_leftover_addresses(processed_addresses, deployed_contracts, replaced_classes, nonces);
    tracing::debug!(
        "Processing {} leftover addresses with nonce or class updates but no storage updates",
        leftover_addresses.len()
    );
    for (address, class_hash, nonce) in leftover_addresses.iter() {
        append_leftover_address_to_blob(*address, *class_hash, *nonce, version, blob_data)?;
    }
    Ok(leftover_addresses.len())
}

/// Appends declared classes count and their data to blob_data
fn append_declared_classes(declared_classes: Vec<DeclaredClassItem>, blob_data: &mut Vec<Felt>) {
    blob_data.push(Felt::from(declared_classes.len()));
    for DeclaredClassItem { class_hash, compiled_class_hash } in declared_classes.into_iter() {
        blob_data.push(class_hash);
        blob_data.push(compiled_class_hash);
    }
}

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
    let state_diff = prepare_state_diff(state_update.state_diff);

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

    let len_storage_diffs = state_diff.storage_diffs.len();

    // Process each storage diff
    process_storage_diffs(
        state_diff.storage_diffs,
        version,
        &mut blob_data,
        &mut processed_addresses,
        &mut deployed_contracts,
        &mut replaced_classes,
        &mut nonces,
    )?;

    // Process leftover nonces (contracts that have nonce updates but no storage updates)
    let leftover_count = process_leftover_addresses(
        &processed_addresses,
        &mut deployed_contracts,
        &mut replaced_classes,
        &mut nonces,
        version,
        &mut blob_data,
    )?;

    // Update the first element with the total number of contracts (original storage diffs and leftover addresses)
    let total_contracts = len_storage_diffs + leftover_count;
    blob_data[0] = Felt::from(total_contracts);
    tracing::debug!("Total contract updates in blob: {}", total_contracts);

    // Add declared classes count and data
    append_declared_classes(state_diff.declared_classes, &mut blob_data);

    tracing::debug!("Created blob data with {} elements", blob_data.len());

    Ok(blob_data)
}

/// Old format (pre-0.13.3)
///
/// Binary encoding layout (252 bits total):
///
/// Bit positions:
/// 251                                                                                   0
/// |                                                                                     |
/// v                                                                                     v
/// ┌───────────────────────────────────────────────────────┬─────┬─────────┬─────────────┐
/// │                 Zero Padding                          │cls  │  nonce  │ num_changes │
/// │                  (123 bits)                           │flg  │(64 bits)│ (64 bits)   │
/// │                  [251:129]                            │[128]│[127:64] │  [63:0]     │
/// └───────────────────────────────────────────────────────┴─────┴─────────┴─────────────┘
///
/// Field breakdown:
/// - Zero Padding (123 bits)  : [251:129] - Padded with zeros
/// - class_flag (1 bit)       : [128]     - Class type flag
/// - nonce (64 bits)          : [127:64]  - Nonce value
/// - num_changes (64 bits)    : [63:0]    - Number of changes
///
/// Total used bits: 123 + 1 + 64 + 64 = 252 bits (exactly)
///
fn encode_da_word_pre_v0_13_3(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
) -> Result<BigUint, JobError> {
    let mut da_word = BigUint::zero();

    // Class flag
    da_word |= if class_flag { BigUint::one() << 128 } else { BigUint::zero() };

    // Nonce
    if let Some(new_nonce) = nonce_change {
        let new_nonce = new_nonce
            .to_u64()
            .ok_or_else(|| JobError::Other(OtherError(eyre!("Nonce value {} exceeds u64 maximum", new_nonce))))?;
        da_word |= BigUint::from(new_nonce) << 64;
    }

    // Number of changes
    da_word |= BigUint::from(num_changes);

    Ok(da_word)
}

/// v0.13.3+ format:
///
/// Binary encoding layout (252 bits total):
///
/// Bit positions:
/// 251                                                                       0
/// |                                                                         |
/// v                                                                         v
/// ┌─────────┬────────────────────────┬──────────────────────────────┬───┬───┐
/// │  Zeros  │       new_nonce        │           n_updates          │n_u│cls│
/// │ Padding │       (64 bits)        │        (8 or 64 bits)        │len│flg│
/// │         │  [129:66] or [73:10]   │       [65:2] or [9:2]        │[1]│[0]│
/// └─────────┴────────────────────────┴──────────────────────────────┴───┴───┘
///
/// Field breakdown:
/// - new_nonce (64 bits)     : [251:188] - Nonce value
/// - n_updates (variable)    : [187:124] (64-bit) or [187:180] (8-bit, then padded by 0s) - Number of updates
/// - n_updates_len (1 bit)   : [1] - Length indicator (1=8-bit, 0=64-bit n_updates)
/// - class_flag (1 bit)      : [0] - Class changed or not
///
/// Total used bits: 64 + (8|64) + 1 + 1 = 74 or 130 bits
/// Remaining bits: 252 - (74|130) = 178 or 122 bits (reserved/padding)
///
fn encode_da_word_v0_13_3_plus(
    class_flag: bool,
    nonce_change: Option<Felt>,
    num_changes: u64,
) -> Result<BigUint, JobError> {
    let mut da_word: BigUint;

    // Determine if we need 8 or 64 bits for num_changes
    let needs_large_updates = num_changes >= 256;

    // Get nonce in 64 bits
    let nonce_felt = nonce_change.unwrap_or(Felt::ZERO);
    let new_nonce = nonce_felt
        .to_u64()
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Nonce value {} exceeds u64 maximum", nonce_felt))))?;

    if needs_large_updates {
        da_word = BigUint::from(new_nonce) << 66;
    } else {
        da_word = (BigUint::from(new_nonce) << 10) | BigUint::from(2u64);
    }

    da_word |= BigUint::from(num_changes) << 2;

    // Class flag
    da_word |= if class_flag { BigUint::one() } else { BigUint::zero() };

    Ok(da_word)
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
    // Parse version to determine format
    let is_gte_v0_13_3 = version >= StarknetVersion::V0_13_3;

    let da_word: BigUint = if is_gte_v0_13_3 {
        encode_da_word_v0_13_3_plus(class_flag, nonce_change, num_changes)?
    } else {
        encode_da_word_pre_v0_13_3(class_flag, nonce_change, num_changes)?
    };

    Ok(Felt::from(da_word))
}

/// Converts a vector of felt into a vector of bigUint
/// The output length depends on the input length (ceil(input_len / BLOB_LEN) * BLOB_LEN)
pub fn convert_to_biguint(elements: &[Felt]) -> Vec<BigUint> {
    let num_blocks = elements.len().div_ceil(BLOB_LEN);
    let output_len = num_blocks * BLOB_LEN;
    let pad_len = output_len - elements.len();

    elements
        .iter()
        .map(|e| BigUint::from_bytes_be(&e.to_bytes_be()))
        .chain(repeat_n(BigUint::zero(), pad_len))
        .collect()
}

/// Converts a vector of felts into blob data (vec of big uint)
/// Returns a vector of blobs
/// A single blob has a fixed size of `BLOB_LEN=4096`
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
