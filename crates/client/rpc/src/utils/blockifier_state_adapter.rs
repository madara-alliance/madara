use std::collections::HashSet;

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use indexmap::IndexMap;
use mc_db::storage_handler::{self, StorageView};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

/// `BlockifierStateAdapter` is only use to re-executing or simulate transactions.
/// None of the setters should therefore change the storage persistently,
/// all changes are temporary stored in the struct and are discarded after the execution
pub struct BlockifierStateAdapter {
    block_number: u64,
    storage_update: IndexMap<ContractAddress, IndexMap<StorageKey, StarkFelt>>,
    nonce_update: IndexMap<ContractAddress, Nonce>,
    class_hash_update: IndexMap<ContractAddress, ClassHash>,
    compiled_class_hash_update: IndexMap<ClassHash, CompiledClassHash>,
    contract_class_update: IndexMap<ClassHash, ContractClass>,
    visited_pcs: IndexMap<ClassHash, HashSet<usize>>,
}

impl BlockifierStateAdapter {
    pub fn new(block_number: u64) -> Self {
        Self {
            block_number,
            storage_update: IndexMap::default(),
            nonce_update: IndexMap::default(),
            class_hash_update: IndexMap::default(),
            compiled_class_hash_update: IndexMap::default(),
            contract_class_update: IndexMap::default(),
            visited_pcs: IndexMap::default(),
        }
    }
}

impl StateReader for BlockifierStateAdapter {
    fn get_storage_at(&mut self, contract_address: ContractAddress, key: StorageKey) -> StateResult<StarkFelt> {
        if contract_address.0.0 == StarkFelt::ONE {
            let block_number = key.0.0.try_into().map_err(|_| StateError::OldBlockHashNotProvided)?;
            match storage_handler::block_hash().get(block_number) {
                Ok(Some(block_hash)) => return Ok(block_hash.into()),
                Ok(None) => return Err(StateError::OldBlockHashNotProvided),
                Err(_) => {
                    return Err(StateError::StateReadError(format!(
                        "Failed to retrieve block hash for block number {}",
                        block_number
                    )));
                }
            }
        }
        match self.storage_update.get(&contract_address).and_then(|storage| storage.get(&key)) {
            Some(value) => Ok(*value),
            None => match storage_handler::contract_storage().get_at(&(contract_address, key), self.block_number) {
                Ok(Some(value)) => Ok(value),
                Ok(None) => Ok(StarkFelt::default()),
                Err(_) => Err(StateError::StateReadError(format!(
                    "Failed to retrieve storage value for contract {} at key {}",
                    contract_address.0.0, key.0.0
                ))),
            },
        }
    }

    fn get_nonce_at(&mut self, contract_address: ContractAddress) -> StateResult<Nonce> {
        match self.nonce_update.get(&contract_address) {
            Some(nonce) => Ok(*nonce),
            None => match storage_handler::contract_data().get_nonce_at(&contract_address, self.block_number) {
                Ok(Some(nonce)) => Ok(nonce),
                Ok(None) => Ok(Nonce::default()),
                Err(_) => Err(StateError::StateReadError(format!(
                    "Failed to retrieve nonce for contract {}",
                    contract_address.0.0
                ))),
            },
        }
    }

    fn get_class_hash_at(&mut self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        match self.class_hash_update.get(&contract_address).cloned() {
            Some(class_hash) => Ok(class_hash),
            None => match storage_handler::contract_data().get_class_hash_at(&contract_address, self.block_number) {
                Ok(Some(class_hash)) => Ok(class_hash),
                Ok(None) => Ok(ClassHash::default()),
                _ => Err(StateError::StateReadError(format!(
                    "failed to retrive class hash for contract address {}",
                    contract_address.0.0
                ))),
            },
        }
    }

    fn get_compiled_contract_class(&mut self, class_hash: ClassHash) -> StateResult<ContractClass> {
        match self.contract_class_update.get(&class_hash) {
            Some(contract_class) => Ok(contract_class.clone()),
            None => match storage_handler::contract_class_data().get(&class_hash) {
                Ok(Some(contract_class_data)) => Ok(contract_class_data.contract_class),
                _ => Err(StateError::UndeclaredClassHash(class_hash)),
            },
        }
    }

    fn get_compiled_class_hash(&mut self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
        match self.compiled_class_hash_update.get(&class_hash) {
            Some(compiled_class_hash) => Ok(*compiled_class_hash),
            None => storage_handler::contract_class_hashes()
                .get(&class_hash)
                .map_err(|_| {
                    StateError::StateReadError(format!(
                        "failed to retrive compiled class hash at class hash {}",
                        class_hash.0
                    ))
                })?
                .ok_or(StateError::UndeclaredClassHash(class_hash)),
        }
    }
}

impl State for BlockifierStateAdapter {
    fn set_storage_at(
        &mut self,
        contract_address: ContractAddress,
        key: StorageKey,
        value: StarkFelt,
    ) -> StateResult<()> {
        self.storage_update.entry(contract_address).or_default().insert(key, value);

        Ok(())
    }

    fn increment_nonce(&mut self, contract_address: ContractAddress) -> StateResult<()> {
        let nonce = self.get_nonce_at(contract_address)?.try_increment().map_err(StateError::StarknetApiError)?;

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

    fn to_state_diff(&mut self) -> CommitmentStateDiff {
        CommitmentStateDiff {
            address_to_class_hash: self.class_hash_update.clone(),
            address_to_nonce: self.nonce_update.clone(),
            storage_updates: self.storage_update.clone(),
            class_hash_to_compiled_class_hash: self.compiled_class_hash_update.clone(),
        }
    }
}

