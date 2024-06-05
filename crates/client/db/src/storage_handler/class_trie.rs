use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::ClassHash;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Poseidon;

use super::{bonsai_identifier, conv_class_key, DeoxysStorageError, StorageType, StorageView, TrieType};
use crate::bonsai_db::BonsaiDb;

pub struct ClassTrieView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>);
pub struct ClassTrieViewMut<'a>(pub(crate) RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>);

impl StorageView for ClassTrieView<'_> {
    type KEY = ClassHash;

    type VALUE = Felt;

    fn get(&self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    fn contains(&self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }
}

impl ClassTrieView<'_> {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CLASS).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Class))
    }
}

impl ClassTrieViewMut<'_> {
    pub fn get(&self, class_hash: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    pub fn contains(&self, class_hash: &ClassHash) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(bonsai_identifier::CLASS, &conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    pub fn insert(&mut self, class_hash: ClassHash, leaf_hash: Felt) -> Result<(), DeoxysStorageError> {
        self.0
            .insert(bonsai_identifier::CLASS, &conv_class_key(&class_hash), &leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Class))
    }

    pub fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::Class, block_number))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CLASS).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Class))
    }

    pub fn update(&mut self, updates: Vec<(&ClassHash, Felt)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_class_key(key);
            self.0
                .insert(bonsai_identifier::CLASS, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        }

        Ok(())
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        self.0.init_tree(bonsai_identifier::CLASS).map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Class))
    }
}
