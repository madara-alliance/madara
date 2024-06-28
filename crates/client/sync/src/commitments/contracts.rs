use std::collections::HashSet;

use blockifier::state::cached_state::CommitmentStateDiff;
use dc_db::storage_handler::{DeoxysStorageError, StorageView};
use dc_db::DeoxysBackend;
use dp_convert::ToFelt;
use rayon::prelude::*;
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::{Pedersen, StarkHash};

/// Calculates the contract trie root
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `block_number`    - The current block number.
///
/// # Returns
///
/// The contract root.
pub fn contract_trie_root(
    backend: &DeoxysBackend,
    csd: &CommitmentStateDiff,
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    // NOTE: handlers implicitely acquire a lock on their respective tries
    // for the duration of their livetimes
    let mut handler_contract = backend.contract_trie_mut();
    let mut handler_storage_trie = backend.contract_storage_trie_mut();

    // First we insert the contract storage changes
    for (contract_address, updates) in csd.storage_updates.iter() {
        handler_storage_trie.init(contract_address)?;

        for (key, value) in updates {
            handler_storage_trie.insert(*contract_address, *key, *value)?;
        }
    }

    // Then we commit them
    handler_storage_trie.commit(block_number)?;

    // We need to initialize the contract trie for each contract that has a class_hash or nonce update
    // to retrieve the corresponding storage root
    for contract_address in csd.address_to_class_hash.keys().chain(csd.address_to_nonce.keys()) {
        if !csd.storage_updates.contains_key(contract_address) {
            // Initialize the storage trie if this contract address does not have storage updates
            handler_storage_trie.init(contract_address)?;
        }
    }

    // We need to calculate the contract_state_leaf_hash for each contract
    // that not appear in the storage_updates but has a class_hash or nonce update
    let all_contract_address: HashSet<ContractAddress> = csd
        .storage_updates
        .keys()
        .chain(csd.address_to_class_hash.keys())
        .chain(csd.address_to_nonce.keys())
        .cloned()
        .collect();

    // Then we compute the leaf hashes retrieving the corresponding storage root
    let updates = all_contract_address
        .iter()
        .par_bridge()
        .map(|contract_address| {
            let storage_root = handler_storage_trie.root(contract_address)?;
            let leaf_hash = contract_state_leaf_hash(backend, csd, contract_address, storage_root)?;

            Ok::<(&ContractAddress, Felt), DeoxysStorageError>((contract_address, leaf_hash))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // then we compute the contract root by applying the changes so far
    handler_contract.update(updates)?;
    handler_contract.commit(block_number)?;

    handler_contract.root()
}

/// Computes the contract state leaf hash
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `contract_address` - The contract address.
/// * `storage_root`     - The storage root of the contract.
///
/// # Returns
///
/// The contract state leaf hash.
fn contract_state_leaf_hash(
    backend: &DeoxysBackend,
    csd: &CommitmentStateDiff,
    contract_address: &ContractAddress,
    storage_root: Felt,
) -> Result<Felt, DeoxysStorageError> {
    let (class_hash, nonce) = class_hash_and_nonce(backend, csd, contract_address)?;

    // computes the contract state leaf hash
    let contract_state_hash = Pedersen::hash(&class_hash, &storage_root);
    let contract_state_hash = Pedersen::hash(&contract_state_hash, &nonce);
    let contract_state_hash = Pedersen::hash(&contract_state_hash, &Felt::ZERO);

    Ok(contract_state_hash)
}

/// Retrieves the class hash and nonce of a contract address
///
/// # Arguments
///
/// * `csd`             - Commitment state diff for the current block.
/// * `contract_address` - The contract address.
///
/// # Returns
///
/// The class hash and nonce of the contract address.
fn class_hash_and_nonce(
    backend: &DeoxysBackend,
    csd: &CommitmentStateDiff,
    contract_address: &ContractAddress,
) -> Result<(Felt, Felt), DeoxysStorageError> {
    let class_hash = match csd.address_to_class_hash.get(contract_address) {
        Some(class_hash) => class_hash.to_felt(),
        None => backend.contract_class_hash().get(&contract_address.to_felt())?.unwrap_or_default(),
    };
    let nonce = match csd.address_to_nonce.get(contract_address) {
        Some(nonce) => nonce.to_felt(),
        None => backend.contract_nonces().get(&contract_address.to_felt())?.unwrap_or_default(),
    };
    Ok((class_hash, nonce))
}
