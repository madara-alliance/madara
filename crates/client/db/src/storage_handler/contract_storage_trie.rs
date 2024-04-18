use std::sync::RwLockReadGuard;

use bitvec::view::AsBits;
use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::api_core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    conv_contract_identifier, conv_contract_storage_key, conv_contract_value, DeoxysStorageError, StorageType, TrieType,
};
use crate::bonsai_db::{BonsaiDb, BonsaiTransaction};
use crate::DeoxysBackend;

pub struct ContractStorageVueMut(pub(crate) BonsaiStorage<BasicId, BonsaiTransaction<'static>, Pedersen>);

pub struct ContractStorageView<'a>(pub(crate) RwLockReadGuard<'a, BonsaiStorage<BasicId, BonsaiDb<'static>, Pedersen>>);

impl ContractStorageVueMut {
    pub fn insert(
        &mut self,
        identifier: &ContractAddress,
        key: &StorageKey,
        value: StarkFelt,
    ) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);
        let value = conv_contract_value(value);

        // self.0.insert(identifier, &key, &value).unwrap();
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .insert(identifier, &key, &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::ContractStorage))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .transactional_commit(BasicId::new(block_number))
        //     .map_err(|_| DeoxysStorageError::TrieCommitError(StorageType::ContractStorage))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractStorage))
    }

    pub fn init(&mut self, identifier: &ContractAddress) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(conv_contract_identifier(identifier))
        //     .map_err(|_| DeoxysStorageError::TrieInitError(StorageType::ContractStorage))?;
        //
        // Ok(())

        DeoxysBackend::bonsai_storage()
            .write()
            .unwrap()
            .init_tree(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::ContractStorage))
    }

    #[deprecated = "Waiting on new transactional state implementations in Bonsai Db"]
    pub fn apply_changes(self) -> Result<(), DeoxysStorageError> {
        // Ok(DeoxysBackend::bonsai_storage()
        //     .write()
        //     .unwrap()
        //     .merge(self.0)
        //     .map_err(|_| DeoxysStorageError::TrieMergeError(StorageType::ContractStorage))?)
        Ok(())
    }

    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        // let identifier = conv_contract_identifier(identifier);
        // let key = conv_contract_storage_key(key);
        //
        // Ok(self
        //     .0
        //     .get(identifier, &key)
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?)

        DeoxysBackend::bonsai_storage()
            .read()
            .unwrap()
            .get(conv_contract_identifier(identifier), &key.0.0.0.as_bits()[5..].to_owned())
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(&self, identifier: &ContractAddress) -> Result<Vec<(Felt, Felt)>, DeoxysStorageError> {
        // Ok(self
        //     .0
        //     .get_key_value_pairs(conv_contract_identifier(identifier))
        //     .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
        //     .into_iter()
        //     .map(|(k, v)| (Felt::from_bytes_be_slice(&k), Felt::from_bytes_be_slice(&v)))
        //     .collect())

        Ok(DeoxysBackend::bonsai_storage()
            .read()
            .unwrap()
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (Felt::from_bytes_be_slice(&k), Felt::from_bytes_be_slice(&v)))
            .collect())
    }
}

impl ContractStorageView<'_> {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
            .get(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(&self, identifier: &ContractAddress) -> Result<Vec<(Felt, Felt)>, DeoxysStorageError> {
        Ok(self
            .0
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (Felt::from_bytes_be_slice(&k), Felt::from_bytes_be_slice(&v)))
            .collect())
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        self.0
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::ContractStorage))
    }
}
