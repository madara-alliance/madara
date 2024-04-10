use core::marker::PhantomData;
use std::collections::{HashMap, HashSet};

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use mc_db::storage::StorageHandler;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

use crate::{Config, Pallet};

/// Empty struct that implements the traits needed by the blockifier/starknet in rust.
///
/// We feed this struct when executing a transaction so that we directly use the substrate storage
/// and not an extra layer that would add overhead.
/// We don't implement those traits directly on the pallet to avoid compilation problems.
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
        let search = StorageHandler::contract_storage().unwrap().get(&contract_address, &key);

        match search {
            Ok(Some(value)) => Ok(StarkFelt(value.to_bytes_be())),
            _ => Ok(StarkFelt::default()),
            // _ => Err(StateError::StateReadError(format!(
            //      "Failed to retrieve storage value for contract {} at key {}",
            //      contract_address.0.0, key.0.0
            // ))),
        }
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        Ok(self.nonce_update.get(&contract_address).cloned().unwrap_or_else(|| Pallet::<T>::nonce(contract_address)))
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        Ok(self
            .class_hash_update
            .get(&contract_address)
            .cloned()
            .unwrap_or_else(|| Pallet::<T>::contract_class_hash_by_address(contract_address)))
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        match self.contract_class_update.get(&class_hash) {
            Some(contract_class) => Ok(contract_class.clone()),
            None => {
                Pallet::<T>::contract_class_by_class_hash(class_hash).ok_or(StateError::UndeclaredClassHash(class_hash))
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
        let nonce = self
            .nonce_update
            .get(&contract_address)
            .cloned()
            .unwrap_or_else(|| Pallet::<T>::nonce(contract_address))
            .try_increment()
            .map_err(StateError::StarknetApiError)?;
        self.nonce_update.insert(contract_address, nonce);

        Ok(())
    }

    fn set_class_hash_at(&mut self, contract_address: ContractAddress, class_hash: ClassHash) -> StateResult<()> {
        self.class_hash_update.insert(contract_address, class_hash);

        Ok(())
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
