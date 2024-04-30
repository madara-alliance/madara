use std::collections::HashMap;

use mp_convert::field_element::FromFieldElement;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateUpdate};
use storage_handler::primitives::contract_class::{
    ClassUpdateWrapper, ContractClassData, ContractClassWrapper, StorageContractClassData,
};

use crate::storage_handler::{self, DeoxysStorageError, StorageView, StorageViewMut};

pub async fn store_state_update(block_number: u64, state_update: StateUpdate) -> Result<(), DeoxysStorageError> {
    let state_diff = state_update.state_diff.clone();
    let nonce_map: HashMap<ContractAddress, Nonce> = state_update
        .state_diff
        .nonces
        .into_iter()
        .map(|NonceUpdate { contract_address, nonce }| {
            (
                ContractAddress(PatriciaKey(StarkFelt::new_unchecked(contract_address.to_bytes_be()))),
                Nonce(StarkFelt::new_unchecked(nonce.to_bytes_be())),
            )
        })
        .collect();

    let (result1, result2, result3, result4) = tokio::join!(
        // Contract address to class hash and nonce update
        async move {
            let handler_contract_data = storage_handler::contract_data_mut();

            let iter_depoyed = state_update.state_diff.deployed_contracts.into_iter().map(
                |DeployedContractItem { address, class_hash }| {
                    (ContractAddress::from_field_element(address), ClassHash::from_field_element(class_hash))
                },
            );
            let iter_replaced = state_update.state_diff.replaced_classes.into_iter().map(
                |ReplacedClassItem { contract_address, class_hash }| {
                    (ContractAddress::from_field_element(contract_address), ClassHash::from_field_element(class_hash))
                },
            );

            iter_depoyed.chain(iter_replaced).for_each(|(contract_address, class_hash)| {
                let class_hash = Some(class_hash);
                let previous_nonce = handler_contract_data.get(&contract_address).unwrap().map(|data| data.nonce);
                let nonce = match previous_nonce.unwrap_or_default().get().copied() {
                    Some(nonce) => Some(nonce),
                    None => nonce_map.get(&contract_address).copied(),
                };

                handler_contract_data.insert(contract_address, (class_hash, nonce)).unwrap()
            });

            handler_contract_data.commit(block_number)
        },
        // Class hash to compiled class hash update
        async move {
            let handler_contract_class_hashes = storage_handler::contract_class_hashes_mut();

            state_update
                .state_diff
                .declared_classes
                .into_iter()
                .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
                    (
                        ClassHash(StarkFelt::new_unchecked(class_hash.to_bytes_be())),
                        CompiledClassHash(StarkFelt::new_unchecked(compiled_class_hash.to_bytes_be())),
                    )
                })
                .for_each(|(class_hash, compiled_class_hash)| {
                    handler_contract_class_hashes.insert(class_hash, compiled_class_hash).unwrap();
                });

            handler_contract_class_hashes.commit(block_number)
        },
        // Block number to state diff update
        async move { storage_handler::block_state_diff().insert(block_number, state_diff) },
        // Contract address to contract storage update
        async move { storage_handler::contract_storage_mut().commit(block_number) }
    );

    match (result1, result2, result3, result4) {
        (Err(err), _, _, _) => Err(err),
        (_, Err(err), _, _) => Err(err),
        (_, _, Err(err), _) => Err(err),
        (_, _, _, Err(err)) => Err(err),
        _ => Ok(()),
    }
}

pub async fn store_class_update(block_number: u64, class_update: ClassUpdateWrapper) -> Result<(), DeoxysStorageError> {
    let handler_contract_class_data_mut = storage_handler::contract_class_data_mut();

    class_update.0.into_iter().for_each(
        |ContractClassData { hash: class_hash, contract_class: contract_class_wrapper }| {
            let ContractClassWrapper { contract: contract_class, abi, sierra_program_length, abi_length } =
                contract_class_wrapper;

            handler_contract_class_data_mut
                .insert(class_hash, StorageContractClassData { contract_class, abi, sierra_program_length, abi_length })
                .unwrap();
        },
    );

    handler_contract_class_data_mut.commit(block_number)
}
