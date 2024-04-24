use std::collections::{HashMap, HashSet};

use blockifier::execution::contract_class::ContractClass;
use blockifier::state::errors::StateError;
use blockifier::state::state_api::{State, StateReader, StateResult};
use mc_db::storage_handler::{self, StorageView};
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;

/// `BlockifierStateAdapter` is only use to re-executing or simulate transactions.
/// None of the setters should therefore change the storage persistently,
/// all changes are temporary stored in the struct and are discarded after the execution
pub struct BlockifierStateAdapter {
    block_number: u64,
    storage_update: HashMap<(ContractAddress, StorageKey), StarkFelt>,
    nonce_update: HashMap<ContractAddress, Nonce>,
    class_hash_update: HashMap<ContractAddress, ClassHash>,
    compiled_class_hash_update: HashMap<ClassHash, CompiledClassHash>,
    contract_class_update: HashMap<ClassHash, ContractClass>,
    visited_pcs: HashMap<ClassHash, HashSet<usize>>,
}

impl BlockifierStateAdapter {
    pub fn new(block_number: u64) -> Self {
        Self {
            block_number,
            storage_update: HashMap::default(),
            nonce_update: HashMap::default(),
            class_hash_update: HashMap::default(),
            compiled_class_hash_update: HashMap::default(),
            contract_class_update: HashMap::default(),
            visited_pcs: HashMap::default(),
        }
    }
}

impl StateReader for BlockifierStateAdapter {
    fn get_storage_at(&self, contract_address: ContractAddress, key: StorageKey) -> StateResult<StarkFelt> {
        match self.storage_update.get(&(contract_address, key)) {
            Some(value) => Ok(*value),
            None => match storage_handler::contract_storage_trie().get_at(&contract_address, &key, self.block_number) {
                Ok(Some(value)) => Ok(StarkFelt(value.to_bytes_be())),
                Ok(None) => Ok(StarkFelt::ZERO),
                Err(_) => Err(StateError::StateReadError(format!(
                    "Failed to retrieve storage value for contract {} at key {}",
                    contract_address.0.0, key.0.0
                ))),
            },
        }
    }

    fn get_nonce_at(&self, contract_address: ContractAddress) -> StateResult<Nonce> {
        match self.nonce_update.get(&contract_address) {
            Some(nonce) => Ok(*nonce),
            None => match storage_handler::contract_data().get_at(&contract_address, self.block_number) {
                Ok(Some(contract_data)) => Ok(contract_data.nonce),
                Ok(None) => Ok(Nonce::default()),
                Err(_) => Err(StateError::StateReadError(format!(
                    "Failed to retrieve nonce for contract {}",
                    contract_address.0.0
                ))),
            },
        }
    }

    fn get_class_hash_at(&self, contract_address: ContractAddress) -> StateResult<ClassHash> {
        match self.class_hash_update.get(&contract_address).cloned() {
            Some(class_hash) => Ok(class_hash),
            None => match storage_handler::contract_data().get_at(&contract_address, self.block_number) {
                Ok(Some(contract_data)) => Ok(contract_data.class_hash),
                _ => Err(StateError::StateReadError(format!(
                    "failed to retrive class hash for contract address {}",
                    contract_address.0.0
                ))),
            },
        }
    }

    fn get_compiled_contract_class(&self, class_hash: ClassHash) -> StateResult<ContractClass> {
        match self.contract_class_update.get(&class_hash) {
            Some(contract_class) => Ok(contract_class.clone()),
            None => match storage_handler::contract_class_data().get(&class_hash) {
                Ok(Some(contract_class_data)) => Ok(contract_class_data.contract_class),
                _ => Err(StateError::UndeclaredClassHash(class_hash)),
            },
        }
    }

    fn get_compiled_class_hash(&self, class_hash: ClassHash) -> StateResult<CompiledClassHash> {
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
