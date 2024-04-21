use core::marker::PhantomData;
use std::collections::{HashMap, HashSet};

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use mc_db::storage_handler::{self, StorageView};
use sp_runtime::traits::UniqueSaturatedInto;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

use crate::{Config, Pallet};

/// `BlockifierStateAdapter` is only use to re-executing or simulate transactions.
/// None of the setters should therefore change the storage persistently,
/// all changes are temporary stored in the struct and are discarded after the execution
pub struct BlockifierStateAdapter<T: Config> {
    storage_update: HashMap<(ContractAddress, StorageKey), StarkFelt>,
    nonce_update: HashMap<ContractAddress, Nonce>,
    class_hash_update: HashMap<ContractAddress, ClassHash>,
    compiled_class_hash_update: HashMap<ClassHash, CompiledClassHash>,
    contract_class_update: HashMap<ClassHash, ContractClass>,
    visited_pcs: HashMap<ClassHash, HashSet<usize>>,
    _phantom: PhantomData<T>,
}

impl<T: Config> Default for BlockifierStateAdapter<T> {
    fn default() -> Self {
        Self {
            storage_update: HashMap::default(),
            nonce_update: HashMap::default(),
            class_hash_update: HashMap::default(),
            compiled_class_hash_update: HashMap::default(),
            contract_class_update: HashMap::default(),
            visited_pcs: HashMap::default(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Config> StateReader for BlockifierStateAdapter<T> {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<StarkFelt> {
        match self.storage_update.get(&(contract_address, key)) {
            Some(value) => Ok(*value),
            None => {
                let block_number =
                    UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number());

                let Ok(handler_storage) = storage_handler::contract_storage_trie() else {
                    return Err(StateError::StateReadError(format!(
                        "Failed to retrieve storage value for contract {} at key {}",
                        contract_address.0.0, key.0.0
                    )));
                };

                let Ok(value) = handler_storage.get_at(&contract_address, &key, block_number) else {
                    return Err(StateError::StateReadError(format!(
                        "Failed to retrieve storage value for contract {} at key {}",
                        contract_address.0.0, key.0.0
                    )));
                };

                match value {
                    Some(value) => Ok(StarkFelt(value.to_bytes_be())),
                    None => Ok(StarkFelt::ZERO),
                }
            }
        }
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        match self.nonce_update.get(&contract_address) {
            Some(nonce) => Ok(*nonce),
            None => {
                let block_number =
                    UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number());

                let Ok(handler_nonce) = storage_handler::nonce() else {
                    return Err(StateError::StateReadError(format!(
                        "Failed to retrive nonce for contract {}",
                        contract_address.0.0
                    )));
                };

                let Ok(nonce) = handler_nonce.get_at(&contract_address, block_number) else {
                    return Err(StateError::StateReadError(format!(
                        "Failed to retrieve nonce for contract {}",
                        contract_address.0.0
                    )));
                };

                match nonce {
                    Some(nonce) => Ok(nonce),
                    None => Ok(Nonce::default()),
                }
            }
        }
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        match self.class_hash_update.get(&contract_address).cloned() {
            Some(class_hash) => Ok(class_hash),
            None => {
                let Ok(handler_class_hash) = storage_handler::class_hash() else {
                    return Err(StateError::StateReadError(format!(
                        "failed to retrive class hash for contract address {}",
                        contract_address.0.0
                    )));
                };

                let Ok(Some(class_hash)) = handler_class_hash.get(&contract_address) else {
                    return Err(StateError::StateReadError(format!(
                        "failed to retrive class hash for contract address {}",
                        contract_address.0.0
                    )));
                };

                Ok(class_hash)
            }
        }
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        match self.contract_class_update.get(&class_hash) {
            Some(contract_class) => Ok(contract_class.clone()),
            None => {
                let block_number =
                    UniqueSaturatedInto::<u64>::unique_saturated_into(frame_system::Pallet::<T>::block_number());

                let Ok(handler_contract_class) = storage_handler::contract_class() else {
                    return Err(StateError::UndeclaredClassHash(class_hash));
                };

                let Ok(Some(contract_class)) = handler_contract_class.get_at(&class_hash, block_number) else {
                    return Err(StateError::UndeclaredClassHash(class_hash));
                };

                Ok(contract_class)
            }
        }
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        match self.compiled_class_hash_update.get(&class_hash) {
            Some(compiled_class_hash) => Ok(*compiled_class_hash),
            None => Pallet::<T>::compiled_class_hash_by_class_hash(class_hash)
                .ok_or(StateError::UndeclaredClassHash(class_hash)),
        }
    }
}

impl<T: Config> State for BlockifierStateAdapter<T> {
    fn set_storage_at(
        &mut self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StarkFelt,
    ) -> StateResult<()> {
        self.storage_update.insert((contract_address, key), value);

        Ok(())
    }

    fn increment_nonce(&mut self, contract_address: ContractAddress) -> StateResult<()> {
        let nonce = self.get_nonce_at(contract_address)?.try_increment().map_err(StateError::StarknetApiError)?;

        self.nonce_update.insert(contract_address, nonce);

        Ok(())
    }

    fn set_class_hash_at(&mut self, contract_address: ContractAddress, class_hash: ClassHash) -> StateResult<()> {
        self.class_hash_update.insert(contract_address, class_hash);

        // TODO: see with @charpa how to implement this with the new storage
        todo!("see with @charpa how to implement this with the new storage")
    }

    fn set_contract_class(&mut self, class_hash: ClassHash, contract_class: ContractClass) -> StateResult<()> {
        self.contract_class_update.insert(class_hash, contract_class);

        Ok(())
    }

    fn set_compiled_class_hash(
        &mut self,
        class_hash: ClassHash,
        compiled_class_hash: CompiledClassHash,
    ) -> StateResult<()> {
        self.compiled_class_hash_update.insert(class_hash, compiled_class_hash);

        Ok(())
    }

    fn add_visited_pcs(&mut self, class_hash: ClassHash, pcs: &HashSet<usize>) {
        self.visited_pcs.entry(class_hash).or_default().extend(pcs);
    }
}
