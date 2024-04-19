use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::{BonsaiStorage, BonsaiStorageConfig};
use starknet_api::api_core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    bonsai_identifier, conv_contract_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut,
    StorageViewRevetible, TrieType,
};
use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

pub struct ContractTrieView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

pub struct ContractTrieViewMut<'a>(
    pub(crate) RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
);

pub struct ContractTrieViewAt {
    pub(crate) storage: BonsaiStorage<BasicId, BonsaiTransaction<'static>, Pedersen>,
    pub(crate) next_id: u64,
}

impl StorageView for ContractTrieView<'_> {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl StorageViewMut for ContractTrieViewMut<'_> {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn insert(&mut self, contract_address: &Self::KEY, leaf_hash: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0
            .insert(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .transactional_commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))
    }
}

impl ContractTrieViewMut<'_> {
    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_contract_key(key);
            self.0
                .insert(bonsai_identifier::CONTRACT, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }

    pub fn init(&mut self) -> Result<(), DeoxysStorageError> {
        self.0.init_tree(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Contract))
    }
}

impl ContractTrieView<'_> {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }
}

impl StorageView for ContractTrieViewAt {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        self.storage
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.storage
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl ContractTrieViewAt {
    fn insert(&mut self, contract_address: &ContractAddress, leaf_hash: &Felt) -> Result<(), DeoxysStorageError> {
        self.storage
            .insert(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))
    }

    fn commit(&mut self) -> Result<(), DeoxysStorageError> {
        let next_id = BasicId::new(self.next_id);
        self.next_id += 1;

        self.storage
            .transactional_commit(next_id)
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))
    }

    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_contract_key(key);
            self.storage
                .insert(bonsai_identifier::CONTRACT, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }
}
