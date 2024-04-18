use std::sync::RwLockReadGuard;

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::api_core::ClassHash;
use starknet_ff::FieldElement;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Poseidon;

use super::{bonsai_identifier, conv_class_key, DeoxysStorageError, StorageType, TrieType};
use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

pub struct ClassViewMut(pub(crate) BonsaiStorage<BasicId, BonsaiTransaction<'static>, Poseidon>);

pub struct ClassTrieView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Poseidon>>);

impl ClassViewMut {
    pub fn update(&mut self, updates: Vec<(&ClassHash, FieldElement)>) -> Result<(), DeoxysStorageError> {
        // for (key, value) in updates {
        //     let key = conv_class_key(key);
        //     let value = Felt::from_bytes_be(&value.to_bytes_be());
        //     self.0
        //         .insert(bonsai_identifier::CLASS, &key, &value)
        //         .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        // }

        let mut lock = DeoxysBackend::bonsai_class().write().unwrap();
        for (key, value) in updates {
            let key = conv_class_key(key);
            let value = Felt::from_bytes_be(&value.to_bytes_be());
            lock.insert(bonsai_identifier::CLASS, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Class))?;
        }

        Ok(())
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Class))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_class()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Class))
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(bonsai_identifier::CLASS)
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_class()
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CLASS)
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Class))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_class()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::Class))?)
        Ok(())
    }

    pub fn get(&self, key: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        // let key = conv_class_key(key);
        //
        // Ok(self
        //     .0
        //     .get(bonsai_identifier::CLASS, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))?)
        DeoxysBackend::bonsai_class()
            .read()
            .unwrap()
            .get(bonsai_identifier::CLASS, &conv_class_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }
}

impl ClassTrieView<'_> {
    pub fn get(&self, key: &ClassHash) -> Result<Option<Felt>, DeoxysStorageError> {
        let key = conv_class_key(key);

        self.0
            .get(bonsai_identifier::CLASS, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Class))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CLASS).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Class))
    }
}
