use std::sync::RwLockReadGuard;

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::api_core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{bonsai_identifier, conv_contract_key, DeoxysStorageError, StorageType, TrieType};
use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

pub struct ContractViewMut(pub(crate) BonsaiStorage<BasicId, BonsaiTransaction<'static>, Pedersen>);

pub struct ContractView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

impl ContractViewMut {
    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        // for (key, value) in updates {
        //     let key = conv_contract_key(key);
        //     self.0
        //         .insert(bonsai_identifier::CONTRACT, &key, &value)
        //         .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        // }

        let mut lock = DeoxysBackend::bonsai_contract().write().unwrap();
        for (key, value) in updates {
            lock.insert(bonsai_identifier::CONTRACT, &conv_contract_key(key), &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::Contract))?;

        DeoxysBackend::bonsai_contract()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(bonsai_identifier::CONTRACT)
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::Class))?;

        DeoxysBackend::bonsai_contract()
            .write()
            .unwrap()
            .init_tree(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Contract))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_contract()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::Contract))?)
        Ok(())
    }

    pub fn get(&self, key: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        // let key = conv_contract_key(key);
        // Ok(self
        //     .0
        //     .get(bonsai_identifier::CONTRACT, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?)

        DeoxysBackend::bonsai_contract()
            .read()
            .unwrap()
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl ContractView<'_> {
    pub fn get(&self, key: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }
}
