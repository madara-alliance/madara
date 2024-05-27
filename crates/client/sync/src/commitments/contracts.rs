use std::collections::HashSet;

use blockifier::state::cached_state::CommitmentStateDiff;
use mc_db::storage_handler::{self, DeoxysStorageError, StorageView};
use mp_felt::Felt252Wrapper;
use mp_hashers::pedersen::PedersenHasher;
use mp_hashers::HasherT;
use rayon::prelude::*;
use starknet_api::core::ContractAddress;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;

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
pub fn contract_trie_root(csd: &CommitmentStateDiff, block_number: u64) -> Result<Felt252Wrapper, DeoxysStorageError> {
    // NOTE: handlers implicitely acquire a lock on their respective tries
    // for the duration of their livetimes
    let mut handler_contract = storage_handler::contract_trie_mut();
    let mut handler_storage_trie = storage_handler::contract_storage_trie_mut();

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
            let leaf_hash = contract_state_leaf_hash(csd, contract_address, storage_root)?;

            Ok::<(&ContractAddress, Felt), DeoxysStorageError>((contract_address, leaf_hash))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // then we compute the contract root by applying the changes so far
    handler_contract.update(updates)?;
    handler_contract.commit(block_number)?;

    Ok(handler_contract.root()?.into())
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
    csd: &CommitmentStateDiff,
    contract_address: &ContractAddress,
    storage_root: Felt,
) -> Result<Felt, DeoxysStorageError> {
    let (class_hash, nonce) = class_hash_and_nonce(csd, contract_address)?;

    let storage_root = FieldElement::from_bytes_be(&storage_root.to_bytes_be()).unwrap();

    // computes the contract state leaf hash
    let contract_state_hash = PedersenHasher::hash_elements(class_hash, storage_root);
    let contract_state_hash = PedersenHasher::hash_elements(contract_state_hash, nonce);
    let contract_state_hash = PedersenHasher::hash_elements(contract_state_hash, FieldElement::ZERO);

    Ok(Felt::from_bytes_be(&contract_state_hash.to_bytes_be()))
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
    csd: &CommitmentStateDiff,
    contract_address: &ContractAddress,
) -> Result<(FieldElement, FieldElement), DeoxysStorageError> {
    let class_hash = csd.address_to_class_hash.get(contract_address);
    let nonce = csd.address_to_nonce.get(contract_address);

    let (class_hash, nonce) = match (class_hash, nonce) {
        (Some(class_hash), Some(nonce)) => (*class_hash, *nonce),
        (Some(class_hash), None) => {
            let nonce = storage_handler::contract_data().get_nonce(contract_address)?.unwrap_or_default();
            (*class_hash, nonce)
        }
        (None, Some(nonce)) => {
            let class_hash = storage_handler::contract_data().get_class_hash(contract_address)?.unwrap_or_default();
            (class_hash, *nonce)
        }
        (None, None) => {
            let contract_data = storage_handler::contract_data().get(contract_address)?.unwrap_or_default();
            let nonce = contract_data.nonce.get().cloned().unwrap_or_default();
            let class_hash = contract_data.class_hash.get().cloned().unwrap_or_default();
            (class_hash, nonce)
        }
    };
    Ok((FieldElement::from_bytes_be(&class_hash.0.0).unwrap(), FieldElement::from_bytes_be(&nonce.0.0).unwrap()))
}
