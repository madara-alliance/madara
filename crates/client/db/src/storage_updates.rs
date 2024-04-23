use std::collections::HashMap;

use mp_block::state_update::StateUpdateWrapper;
use mp_contract::class::{
    ClassUpdateWrapper, ContractClassData, ContractClassWrapper, StorageContractClassData, StorageContractData,
};
use rayon::prelude::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use tokio::task;

use crate::storage_handler::{self, DeoxysStorageError, StorageViewMut};

pub async fn store_state_update(block_number: u64, state_update: StateUpdateWrapper) -> Result<(), DeoxysStorageError> {
    let nonce_map: HashMap<ContractAddress, Nonce> = state_update
        .state_diff
        .nonces
        .into_iter()
        .map(|(contract_address, nonce)| {
            (
                ContractAddress(PatriciaKey(StarkFelt(contract_address.0.to_bytes_be()))),
                Nonce(StarkFelt(nonce.0.to_bytes_be())),
            )
        })
        .collect();

    let (result1, result2) = tokio::join!(
        task::spawn_blocking(move || {
            let handler_contract_data = storage_handler::contract_data_mut();
            state_update
                .state_diff
                .deployed_contracts
                .par_iter()
                .chain(state_update.state_diff.replaced_classes.par_iter())
                .map(|contract| (ContractAddress(contract.address.into()), ClassHash(contract.class_hash.into())))
                .for_each(|(contract_address, class_hash)| {
                    handler_contract_data
                        .insert(
                            contract_address,
                            StorageContractData {
                                class_hash,
                                nonce: nonce_map.get(&contract_address).cloned().unwrap_or_default(),
                            },
                        )
                        .unwrap()
                });
            handler_contract_data.commit(block_number)
        }),
        task::spawn_blocking(move || {
            let handler_contract_class_hashes = storage_handler::contract_class_hashes_mut();
            state_update
                .state_diff
                .declared_classes
                .into_par_iter()
                .map(|declared_class| {
                    (
                        ClassHash(declared_class.class_hash.into()),
                        CompiledClassHash(declared_class.compiled_class_hash.into()),
                    )
                })
                .for_each(|(class_hash, compiled_class_hash)| {
                    handler_contract_class_hashes.insert(class_hash, compiled_class_hash).unwrap();
                });
            handler_contract_class_hashes.commit(block_number)
        })
    );

    match (result1.unwrap(), result2.unwrap()) {
        (Err(err), _) => Err(err),
        (_, Err(err)) => Err(err),
        _ => Ok(()),
    }
}

pub async fn store_class_update(block_number: u64, class_update: ClassUpdateWrapper) -> Result<(), DeoxysStorageError> {
    task::spawn_blocking(move || {
        let handler_contract_cladd_data_mut = storage_handler::contract_class_data_mut();
        class_update.0.into_par_iter().for_each(
            |ContractClassData { hash: class_hash, contract_class: contract_class_wrapper }| {
                let ContractClassWrapper { contract: contract_class, abi } = contract_class_wrapper;

                handler_contract_cladd_data_mut
                    .insert(class_hash, StorageContractClassData { contract_class, abi })
                    .unwrap();
            },
        );

        handler_contract_cladd_data_mut.commit(block_number)
    })
    .await
    .unwrap()
}
