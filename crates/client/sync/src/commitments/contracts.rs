use std::collections::HashSet;

use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use dc_db::storage_handler::{DeoxysStorageError, StorageType};
use dc_db::DeoxysBackend;
use dp_block::{BlockId, BlockTag};
use dp_convert::ToFelt;
use indexmap::IndexMap;
use rayon::prelude::*;
use starknet_api::core::{ClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
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
    address_to_class_hash: IndexMap<ContractAddress, ClassHash>,
    address_to_nonce: IndexMap<ContractAddress, Nonce>,
    storage_updates: IndexMap<ContractAddress, IndexMap<StorageKey, StarkFelt>>,
    block_number: u64,
) -> Result<Felt, DeoxysStorageError> {
    // We need to calculate the contract_state_leaf_hash for each contract
    // that do not appear in the storage_updates but has a class_hash or nonce update
    let all_contract_address: HashSet<ContractAddress> =
        storage_updates.keys().chain(address_to_class_hash.keys()).chain(address_to_nonce.keys()).cloned().collect();

    let mut contract_storage_trie = backend.contract_storage_trie();

    // First we insert the contract storage changes
    for (contract_address, updates) in storage_updates {
        for (key, value) in updates {
            contract_storage_trie
                .insert(contract_address.0.key().bytes(), &key.0.key().bytes().as_bits()[5..], &value.to_felt())
                .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?;
        }
    }

    // Then we commit them
    contract_storage_trie
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?;

    let mut contract_trie = backend.contract_trie();

    // Then we compute the leaf hashes retrieving the corresponding storage root
    let updates: Vec<_> = all_contract_address
        .into_par_iter()
        .map(|contract_address| {
            let storage_root = contract_storage_trie
                .root_hash(contract_address.0.key().bytes())
                .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?;
            let leaf_hash = contract_state_leaf_hash(
                backend,
                &address_to_class_hash,
                &address_to_nonce,
                &contract_address,
                storage_root,
            )?;

            Ok((contract_address, leaf_hash))
        })
        .collect::<Result<_, DeoxysStorageError>>()?;

    for (key, value) in updates {
        contract_trie
            .insert(&[], &key.0.key().bytes().as_bits()[5..], &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
    }

    contract_trie
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?;
    let root_hash =
        contract_trie.root_hash(&[]).map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?;
    Ok(root_hash)
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
    address_to_class_hash: &IndexMap<ContractAddress, ClassHash>,
    address_to_nonce: &IndexMap<ContractAddress, Nonce>,
    contract_address: &ContractAddress,
    storage_root: Felt,
) -> Result<Felt, DeoxysStorageError> {
    let (class_hash, nonce) = class_hash_and_nonce(backend, address_to_class_hash, address_to_nonce, contract_address)?;

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
    address_to_class_hash: &IndexMap<ContractAddress, ClassHash>,
    address_to_nonce: &IndexMap<ContractAddress, Nonce>,
    contract_address: &ContractAddress,
) -> Result<(Felt, Felt), DeoxysStorageError> {
    let class_hash = match address_to_class_hash.get(contract_address) {
        Some(class_hash) => class_hash.to_felt(),
        None => {
            // TODO: This is suspect: if class hash not found in the trie, we default to class hash zero?
            backend
                .get_contract_class_hash_at(&BlockId::Tag(BlockTag::Latest), &contract_address.to_felt())?
                .unwrap_or(Felt::ZERO)
        }
    };
    let nonce = match address_to_nonce.get(contract_address) {
        Some(nonce) => nonce.to_felt(),
        None => backend
            .get_contract_nonce_at(&BlockId::Tag(BlockTag::Latest), &contract_address.to_felt())?
            .unwrap_or(Felt::ZERO),
    };
    Ok((class_hash, nonce))
}
