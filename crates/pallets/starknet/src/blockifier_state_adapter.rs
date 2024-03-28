use alloc::collections::{BTreeMap, BTreeSet};
use core::marker::PhantomData;

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::cached_state::{CommitmentStateDiff, StateChangesCount};
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use indexmap::IndexMap;
use mp_felt::Felt252Wrapper;
use mp_state::StateChanges;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_crypto::FieldElement;

use crate::types::ContractStorageKey;
use crate::{Config, Pallet};

/// Empty struct that implements the traits needed by the blockifier/starknet in rust.
///
/// We feed this struct when executing a transaction so that we directly use the substrate storage
/// and not an extra layer that would add overhead.
/// We don't implement those traits directly on the pallet to avoid compilation problems.
pub struct BlockifierStateAdapter<T: Config> {
    storage_update: BTreeMap<ContractAddress, Vec<(StorageKey, StarkFelt)>>,
    class_hash_update: usize,
    compiled_class_hash_update: usize,
    _phantom: PhantomData<T>,
}

impl<T> StateChanges for BlockifierStateAdapter<T>
where
    T: Config,
{
    fn count_state_changes(&self) -> StateChangesCount {
        let keys = self.storage_update.keys();
        let n_contract_updated = BTreeSet::from_iter(keys.clone()).len();
        StateChangesCount {
            n_modified_contracts: n_contract_updated,
            n_storage_updates: keys.len(),
            n_class_hash_updates: self.class_hash_update,
            n_compiled_class_hash_updates: self.compiled_class_hash_update,
        }
    }
}

impl<T: Config> Default for BlockifierStateAdapter<T> {
    fn default() -> Self {
        Self {
            storage_update: BTreeMap::default(),
            class_hash_update: usize::default(),
            compiled_class_hash_update: usize::default(),
            _phantom: PhantomData,
        }
    }
}

impl<T: Config> StateReader for BlockifierStateAdapter<T> {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<StarkFelt> {
        let search = Pallet::<T>::storage(contract_address).into_iter().find(|(storage_key, _)| key == *storage_key);

        match search {
            Some((_, value)) => Ok(value),
            None => Err(StateError::StateReadError(format!(
                "Failed to retrieve storage value for contract {} at key {}",
                contract_address.0.0, key.0.0
            ))),
        }
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        Ok(Pallet::<T>::nonce(contract_address))
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        Ok(Pallet::<T>::contract_class_hash_by_address(contract_address))
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        Pallet::<T>::contract_class_by_class_hash(class_hash).ok_or(StateError::UndeclaredClassHash(class_hash))
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        Pallet::<T>::compiled_class_hash_by_class_hash(class_hash).ok_or(StateError::UndeclaredClassHash(class_hash))
    }
}

impl<T: Config> State for BlockifierStateAdapter<T> {
    fn set_storage_at(
        &mut self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StarkFelt,
    ) -> StateResult<()> {
        self.storage_update.insert(contract_address, vec![(key, value)]);
        crate::StorageView::<T>::insert(contract_address, vec![(key, value)]);
        Ok(())
    }

    fn increment_nonce(&mut self, contract_address: ContractAddress) -> StateResult<()> {
        let current_nonce = Pallet::<T>::nonce(contract_address);
        let current_nonce: FieldElement = Felt252Wrapper::from(current_nonce.0).into();
        let new_nonce: Nonce = Felt252Wrapper(current_nonce + FieldElement::ONE).into();

        crate::Nonces::<T>::insert(contract_address, new_nonce);

        Ok(())
    }

    fn set_class_hash_at(&mut self, contract_address: ContractAddress, class_hash: ClassHash) -> StateResult<()> {
        self.class_hash_update += 1;

        crate::ContractClassHashes::<T>::insert(contract_address, class_hash);

        Ok(())
    }

    fn set_contract_class(&mut self, class_hash: ClassHash, contract_class: ContractClass) -> StateResult<()> {
        crate::ContractClasses::<T>::insert(class_hash, contract_class);

        Ok(())
    }

    fn set_compiled_class_hash(
        &mut self,
        class_hash: ClassHash,
        compiled_class_hash: CompiledClassHash,
    ) -> StateResult<()> {
        self.compiled_class_hash_update += 1;
        crate::CompiledClassHashes::<T>::insert(class_hash, compiled_class_hash);

        Ok(())
    }

    fn add_visited_pcs(&mut self, class_hash: ClassHash, pcs: &std::collections::HashSet<usize>) {
        // TODO
        // This should not be part of the trait.
        // Hopefully it will be fixed upstream
        unreachable!()
    }
}
