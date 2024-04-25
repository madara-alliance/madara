use std::collections::HashMap;
use std::sync::Arc;

use mp_contract::class::{
    ClassUpdateWrapper, ContractClassData, ContractClassWrapper, StorageContractClassData, StorageContractData,
};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateUpdate};
use tokio::task::{self, spawn_blocking};

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

    let (result1, result2, result3) = tokio::join!(
        async move {
            let handler_contract_data = Arc::new(storage_handler::contract_data_mut());
            let handler_contract_data_1 = Arc::clone(&handler_contract_data);

            let iter_depoyed = state_update.state_diff.deployed_contracts.into_par_iter().map(
                |DeployedContractItem { address, class_hash }| {
                    (
                        ContractAddress(PatriciaKey(StarkFelt::new_unchecked(address.to_bytes_be()))),
                        ClassHash(StarkFelt::new_unchecked(class_hash.to_bytes_be())),
                    )
                },
            );
            let iter_replaced = state_update.state_diff.replaced_classes.into_par_iter().map(
                |ReplacedClassItem { contract_address, class_hash }| {
                    (
                        ContractAddress(PatriciaKey(StarkFelt::new_unchecked(contract_address.to_bytes_be()))),
                        ClassHash(StarkFelt::new_unchecked(class_hash.to_bytes_be())),
                    )
                },
            );

            task::spawn_blocking(move || {
                iter_depoyed.chain(iter_replaced).for_each(|(contract_address, class_hash)| {
                    let previous_nonce = handler_contract_data_1.get(&contract_address).unwrap().map(|data| data.nonce);

                    handler_contract_data_1
                        .insert(
                            contract_address,
                            StorageContractData {
                                class_hash,
                                nonce: match nonce_map.get(&contract_address) {
                                    Some(nonce) => *nonce,
                                    None => previous_nonce.unwrap_or_default(),
                                },
                            },
                        )
                        .unwrap()
                });
            })
            .await
            .unwrap();

            Arc::try_unwrap(handler_contract_data).expect("arc should not be aliased").commit(block_number).await
        },
        async move {
            let handler_contract_class_hashes = Arc::new(storage_handler::contract_class_hashes_mut());
            let handler_contract_class_hashes_1 = Arc::clone(&handler_contract_class_hashes);

            task::spawn_blocking(move || {
                state_update
                    .state_diff
                    .declared_classes
                    .into_par_iter()
                    .map(|DeclaredClassItem { class_hash, compiled_class_hash }| {
                        (
                            ClassHash(StarkFelt::new_unchecked(class_hash.to_bytes_be())),
                            CompiledClassHash(StarkFelt::new_unchecked(compiled_class_hash.to_bytes_be())),
                        )
                    })
                    .for_each(|(class_hash, compiled_class_hash)| {
                        handler_contract_class_hashes_1.insert(class_hash, compiled_class_hash).unwrap();
                    });
            })
            .await
            .unwrap();

            Arc::try_unwrap(handler_contract_class_hashes)
                .expect("arc should not be aliased")
                .commit(block_number)
                .await
        },
        async move { storage_handler::block_state_diff().insert(block_number, state_diff) }
    );

    match (result1, result2, result3) {
        (Err(err), _, _) => Err(err),
        (_, Err(err), _) => Err(err),
        (_, _, Err(err)) => Err(err),
        _ => Ok(()),
    }
}

pub async fn store_class_update(block_number: u64, class_update: ClassUpdateWrapper) -> Result<(), DeoxysStorageError> {
    let handler_contract_class_data_mut = Arc::new(storage_handler::contract_class_data_mut());
    let handler_contract_class_data_mut_1 = Arc::clone(&handler_contract_class_data_mut);

    spawn_blocking(move || {
        class_update.0.into_par_iter().for_each(
            |ContractClassData { hash: class_hash, contract_class: contract_class_wrapper }| {
                let ContractClassWrapper { contract: contract_class, abi, sierra_program_length, abi_length } =
                    contract_class_wrapper;

                handler_contract_class_data_mut_1
                    .insert(
                        class_hash,
                        StorageContractClassData { contract_class, abi, sierra_program_length, abi_length },
                    )
                    .unwrap();
            },
        );
    })
    .await
    .unwrap();

    Arc::try_unwrap(handler_contract_class_data_mut).expect("arch should not be aliased").commit(block_number).await
}
