use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::{ContractAddress, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    conv_contract_identifier, conv_contract_storage_key, conv_contract_value, DeoxysStorageError, StorageType, TrieType,
};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct ContractStorageTrieView;
pub struct ContractStorageTrieViewMut;

impl ContractStorageTrieView {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db()
            .get(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_at(
        &self,
        identifier: &ContractAddress,
        key: &StorageKey,
        block_number: u64,
    ) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db()
            .get_at(identifier, &key, BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(
        &self,
        identifier: &ContractAddress,
    ) -> Result<Vec<(StorageKey, StarkFelt)>, DeoxysStorageError> {
        Ok(contract_storage_trie_db()
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (StorageKey(PatriciaKey(starkfelt(&k))), starkfelt(&v)))
            .collect())
    }

    pub fn contains(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<bool, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db()
            .contains(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        contract_storage_trie_db()
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::ContractStorage))
    }
}

impl ContractStorageTrieViewMut {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db_mut()
            .get(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_at(
        &self,
        identifier: &ContractAddress,
        key: &StorageKey,
        block_number: u64,
    ) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db_mut()
            .get_at(identifier, &key, BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(
        &self,
        identifier: &ContractAddress,
    ) -> Result<Vec<(StorageKey, StarkFelt)>, DeoxysStorageError> {
        Ok(contract_storage_trie_db_mut()
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (StorageKey(PatriciaKey(starkfelt(&k))), starkfelt(&v)))
            .collect())
    }

    pub fn contains(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<bool, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        contract_storage_trie_db_mut()
            .contains(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        contract_storage_trie_db_mut()
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::ContractStorage))
    }
    pub fn insert(
        &mut self,
        identifier: &ContractAddress,
        key: &StorageKey,
        value: StarkFelt,
    ) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);
        let value = conv_contract_value(value);

        contract_storage_trie_db_mut()
            .insert(identifier, &key, &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::ContractStorage))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        contract_storage_trie_db_mut()
            .transactional_commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractStorage))
    }

    pub fn init(&mut self, identifier: &ContractAddress) -> Result<(), DeoxysStorageError> {
        contract_storage_trie_db_mut()
            .init_tree(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::ContractStorage))
    }

    pub fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        contract_storage_trie_db_mut()
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractStorage, block_number))
    }
}

fn starkfelt(bytes: &[u8]) -> StarkFelt {
    StarkFelt(Felt::from_bytes_be_slice(bytes).to_bytes_be())
}

fn contract_storage_trie_db<'a>() -> RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
    DeoxysBackend::bonsai_storage().read().unwrap()
}

fn contract_storage_trie_db_mut<'a>() -> RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>> {
    DeoxysBackend::bonsai_storage().write().unwrap()
}
