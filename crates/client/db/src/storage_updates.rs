use std::collections::HashMap;

use dp_convert::ToStarkFelt;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use starknet_api::core::Nonce;
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateUpdate,
    StorageEntry,
};
use starknet_types_core::felt::Felt;

use crate::storage_handler::primitives::contract_class::{
    ClassUpdateWrapper, ContractClassData, ContractClassWrapper, StorageContractClassData,
};
use crate::storage_handler::{DeoxysStorageError, StorageViewMut};
use crate::DeoxysBackend;

pub fn store_state_update(
    backend: &DeoxysBackend,
    block_number: u64,
    state_update: StateUpdate,
) -> Result<(), DeoxysStorageError> {
    let state_diff = state_update.state_diff.clone();
    let nonce_map: HashMap<Felt, Nonce> = state_update
        .state_diff
        .nonces
        .into_iter()
        .map(|NonceUpdate { contract_address, nonce }| (contract_address, Nonce(nonce.to_stark_felt())))
        .collect();

    log::debug!("💾 update state: block_number: {}", block_number);

    // Contract address to class hash and nonce update
    let task_1 = || {
        let handler_contract_data_class = backend.contract_class_hash_mut();
        let handler_contract_data_nonces = backend.contract_nonces_mut();

        state_update
            .state_diff
            .deployed_contracts
            .into_iter()
            .map(|DeployedContractItem { address, class_hash }| (address, class_hash))
            .try_for_each(|(contract_address, class_hash)| -> Result<(), DeoxysStorageError> {
                handler_contract_data_class.insert(contract_address, class_hash)?;
                // insert nonces for contracts that were deployed in this block and do not have a nonce
                if !nonce_map.contains_key(&contract_address) {
                    handler_contract_data_nonces.insert(contract_address, Nonce::default())?;
                }
                Ok(())
            })?;

        state_update
            .state_diff
            .replaced_classes
            .into_iter()
            .map(|ReplacedClassItem { contract_address, class_hash }| (contract_address, class_hash))
            .try_for_each(|(contract_address, class_hash)| -> Result<(), DeoxysStorageError> {
                handler_contract_data_class.insert(contract_address, class_hash)?;
                Ok(())
            })?;

        // insert nonces for contracts that were not deployed or replaced in this block
        nonce_map.into_iter().for_each(|(contract_address, nonce)| {
            handler_contract_data_nonces.insert(contract_address, nonce).unwrap();
        });

        handler_contract_data_class.commit(block_number)?;
        log::debug!("committed contract_data_class");
        handler_contract_data_nonces.commit(block_number)?;
        log::debug!("committed contract_data_nonces");
        Ok(())
    };

    // Class hash to compiled class hash update
    let task_2 = || {
        let handler_contract_class_hashes = backend.contract_class_hashes_mut();

        state_update
            .state_diff
            .declared_classes
            .into_iter()
            .map(|DeclaredClassItem { class_hash, compiled_class_hash }| (class_hash, compiled_class_hash))
            .for_each(|(class_hash, compiled_class_hash)| {
                handler_contract_class_hashes.insert(class_hash, compiled_class_hash).unwrap();
            });

        handler_contract_class_hashes.commit(block_number)?;
        log::debug!("committed contract_class_hashes");
        Ok(())
    };

    // Block number to state diff update
    let task_3 = || backend.block_state_diff().insert(block_number, state_diff);

    let (result1, (result2, result3)) = rayon::join(task_1, || rayon::join(task_2, task_3));

    result1.and(result2).and(result3)
}

pub fn store_class_update(
    backend: &DeoxysBackend,
    block_number: u64,
    class_update: ClassUpdateWrapper,
) -> Result<(), DeoxysStorageError> {
    let handler_contract_class_data_mut = backend.contract_class_data_mut();

    class_update.0.into_iter().for_each(
        |ContractClassData { hash: class_hash, contract_class: contract_class_wrapper }| {
            let ContractClassWrapper { contract_class, abi, sierra_program_length, abi_length } =
                contract_class_wrapper;

            handler_contract_class_data_mut
                .insert(
                    class_hash,
                    StorageContractClassData { contract_class, abi, sierra_program_length, abi_length, block_number },
                )
                .unwrap();
        },
    );

    handler_contract_class_data_mut.commit(block_number)?;
    log::debug!("committed contract_class_data_mut");
    Ok(())
}

pub fn store_key_update(
    backend: &DeoxysBackend,
    block_number: u64,
    storage_diffs: &[ContractStorageDiffItem],
) -> Result<(), DeoxysStorageError> {
    let handler_storage = backend.contract_storage_mut();

    storage_diffs.into_par_iter().try_for_each(|ContractStorageDiffItem { address, storage_entries }| {
        storage_entries.iter().try_for_each(|StorageEntry { key, value }| -> Result<(), DeoxysStorageError> {
            handler_storage.insert((*address, *key), *value)
        })
    })?;

    handler_storage.commit(block_number)?;
    log::debug!("committed key_update");

    Ok(())
}
