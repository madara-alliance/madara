use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::ClassHash;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Poseidon;

use super::{
    bonsai_identifier, conv_class_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut,
    StorageViewRevetible, TrieType,
};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct ClassTrieView;

pub struct ClassTrieViewMut;

impl StorageView for ClassTrieView {
    type KEY = ClassHash;

    type VALUE = Felt;

    fn get(self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        class_trie_db()
            .get(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    fn get_at(self, class_hash: &Self::KEY, block_number: u64) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        class_trie_db()
            .get_at(bonsai_identifier::CLASS, &conv_class_key(class_hash), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    fn contains(self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        class_trie_db()
            .contains(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }
}

impl StorageView for ClassTrieViewMut {
    type KEY = ClassHash;

    type VALUE = Felt;

    fn get(self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        class_trie_db_mut()
            .get(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    fn get_at(self, class_hash: &Self::KEY, block_number: u64) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        class_trie_db_mut()
            .get_at(bonsai_identifier::CLASS, &conv_class_key(class_hash), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    fn contains(self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        class_trie_db_mut()
            .contains(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }
}

impl StorageViewMut for ClassTrieViewMut {
    type KEY = ClassHash;

    type VALUE = Felt;

    fn insert(&mut self, class_hash: &Self::KEY, leaf_hash: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        class_trie_db_mut()
            .insert(bonsai_identifier::CLASS, &conv_class_key(class_hash), leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        class_trie_db_mut()
            .transactional_commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Class))
    }
}

impl StorageViewRevetible for ClassTrieViewMut {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        class_trie_db_mut()
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::Class, block_number))
    }
}

impl ClassTrieViewMut {
    pub fn root(self) -> Result<Felt, DeoxysStorageError> {
        class_trie_db_mut()
            .root_hash(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Class))
    }

    pub fn update(&mut self, updates: Vec<(&ClassHash, FieldElement)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_class_key(key);
            let value = Felt::from_bytes_be(&value.to_bytes_be());
            class_trie_db_mut()
                .insert(bonsai_identifier::CLASS, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        }

        Ok(())
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        class_trie_db_mut()
            .init_tree(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Class))
    }
}

impl ClassTrieView {
    pub fn root(self) -> Result<Felt, DeoxysStorageError> {
        class_trie_db()
            .root_hash(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Class))
    }
}

fn class_trie_db<'a>() -> RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>> {
    DeoxysBackend::bonsai_class().read().unwrap()
}

fn class_trie_db_mut<'a>() -> RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>> {
    DeoxysBackend::bonsai_class().write().unwrap()
}
