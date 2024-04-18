use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::api_core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    bonsai_identifier, conv_contract_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut, TrieType,
};
use crate::bonsai_db::BonsaiDb;

pub struct ContractTrieView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

pub struct ContractTrieViewMut<'a>(
    pub(crate) RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
);

impl StorageView for ContractTrieView<'_> {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        Ok(self
            .0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?)
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self
            .0
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))?)
    }
}

impl StorageViewMut for ContractTrieViewMut<'_> {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn insert(&mut self, contract_address: &Self::KEY, leaf_hash: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .insert(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address), leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?)
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .transactional_commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))?)
    }

    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::Contract, block_number))?)
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
        Ok(self
            .0
            .init_tree(bonsai_identifier::CONTRACT)
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::Contract))?)
    }
}

impl ContractTrieView<'_> {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }
}
