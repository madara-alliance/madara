use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    bonsai_identifier, conv_contract_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut, TrieType,
};
use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

pub struct ContractTrieView;
pub struct ContractTrieViewMut;

impl StorageView for ContractTrieView {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        contract_trie_db()
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn get_at(
        self,
        contract_address: &Self::KEY,
        block_number: u64,
    ) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        contract_trie_db()
            .get_at(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        contract_trie_db()
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl ContractTrieView {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        contract_trie_db()
            .root_hash(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }
}

impl StorageView for ContractTrieViewMut {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        contract_trie_db_mut()
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn get_at(
        self,
        contract_address: &Self::KEY,
        block_number: u64,
    ) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        contract_trie_db_mut()
            .get_at(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        contract_trie_db_mut()
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl StorageViewMut for ContractTrieViewMut {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn insert(&mut self, contract_address: &Self::KEY, leaf_hash: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        contract_trie_db_mut()
            .insert(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        contract_trie_db_mut()
            .transactional_commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))
    }
}

impl ContractTrieViewMut {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        contract_trie_db_mut()
            .root_hash(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }

    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_contract_key(key);
            contract_trie_db_mut()
                .insert(bonsai_identifier::CONTRACT, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }
}

fn contract_trie_db<'a>() -> RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
    DeoxysBackend::bonsai_contract().read().unwrap()
}

fn contract_trie_db_mut<'a>() -> RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
    DeoxysBackend::bonsai_contract().write().unwrap()
}
