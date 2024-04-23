use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::prelude::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::ContractAddress;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{bonsai_identifier, DeoxysStorageError, StorageType, StorageView, TrieType};
use crate::bonsai_db::BonsaiDb;

pub struct ContractTrieView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);
pub struct ContractTrieViewMut<'a>(
    pub(crate) RwLockWriteGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>,
);

impl StorageView for ContractTrieView<'_> {
    type KEY = ContractAddress;

    type VALUE = Felt;

    fn get(&self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    fn contains(&self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }
}

impl ContractTrieView<'_> {
    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }
}

impl ContractTrieViewMut<'_> {
    pub fn get(&self, contract_address: &ContractAddress) -> Result<Option<Felt>, DeoxysStorageError> {
        self.0
            .get(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    pub fn contains(&self, contract_address: &ContractAddress) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(bonsai_identifier::CONTRACT, &conv_contract_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::Contract))
    }

    pub fn insert(&mut self, contract_address: ContractAddress, leaf_hash: Felt) -> Result<(), DeoxysStorageError> {
        self.0
            .insert(bonsai_identifier::CONTRACT, &conv_contract_key(&contract_address), &leaf_hash)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::Contract))
    }

    pub fn root(&self) -> Result<Felt, DeoxysStorageError> {
        self.0.root_hash(bonsai_identifier::CONTRACT).map_err(|_| DeoxysStorageError::TrieRootError(TrieType::Contract))
    }

    pub fn update(&mut self, updates: Vec<(&ContractAddress, Felt)>) -> Result<(), DeoxysStorageError> {
        for (key, value) in updates {
            let key = conv_contract_key(key);
            self.0
                .insert(bonsai_identifier::CONTRACT, &key, &value)
                .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::Contract))?
        }

        Ok(())
    }
}

fn conv_contract_key(key: &ContractAddress) -> BitVec<u8, Msb0> {
    key.0.0.0.as_bits()[5..].to_owned()
}
