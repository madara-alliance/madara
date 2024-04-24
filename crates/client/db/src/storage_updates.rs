use std::collections::HashMap;
use std::sync::Arc;

use mp_block::state_update::StateUpdateWrapper;
use mp_contract::class::{
    ClassUpdateWrapper, ContractClassData, ContractClassWrapper, StorageContractClassData, StorageContractData,
};
use rayon::prelude::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce, PatriciaKey};
use starknet_api::hash::StarkFelt;
use tokio::task::{self, spawn_blocking};

use crate::storage_handler::{self, DeoxysStorageError, StorageView, StorageViewMut};

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
        async move {
            let handler_contract_data = Arc::new(storage_handler::contract_data_mut());
            let handler_contract_data_1 = Arc::clone(&handler_contract_data);

            task::spawn_blocking(move || {
                state_update
                    .state_diff
                    .deployed_contracts
                    .par_iter()
                    .chain(state_update.state_diff.replaced_classes.par_iter())
                    .map(|contract| (ContractAddress(contract.address.into()), ClassHash(contract.class_hash.into())))
                    .for_each(|(contract_address, class_hash)| {
                        let previous_nonce =
                            handler_contract_data_1.get(&contract_address).unwrap().map(|data| data.nonce);

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
                    .map(|declared_class| {
                        (
                            ClassHash(declared_class.class_hash.into()),
                            CompiledClassHash(declared_class.compiled_class_hash.into()),
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
    );

    match (result1, result2) {
        (Err(err), _) => Err(err),
        (_, Err(err)) => Err(err),
        _ => Ok(()),
    }
}

pub async fn store_class_update(block_number: u64, class_update: ClassUpdateWrapper) -> Result<(), DeoxysStorageError> {
    let handler_contract_class_data_mut = Arc::new(storage_handler::contract_class_data_mut());
    let handler_contract_class_data_mut_1 = Arc::clone(&handler_contract_class_data_mut);

    spawn_blocking(move || {
        class_update.0.into_par_iter().for_each(
            |ContractClassData { hash: class_hash, contract_class: contract_class_wrapper }| {
                let ContractClassWrapper { contract: contract_class, abi } = contract_class_wrapper;

                handler_contract_class_data_mut_1
                    .insert(class_hash, StorageContractClassData { contract_class, abi })
                    .unwrap();
            },
        );
    })
    .await
    .unwrap();

    Arc::try_unwrap(handler_contract_class_data_mut).expect("arch should not be aliased").commit(block_number).await
}
