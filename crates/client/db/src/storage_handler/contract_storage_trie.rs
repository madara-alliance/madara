use bonsai_trie::id::BasicId;
use bonsai_trie::BonsaiStorage;
use starknet_api::core::ContractAddress;
use starknet_api::hash::StarkFelt;
use starknet_api::state::StorageKey;
use starknet_types_core::felt::Felt;
use starknet_types_core::hash::Pedersen;

use super::{
    conv_contract_identifier, conv_contract_storage_key, conv_contract_value, DeoxysStorageError, StorageType, TrieType,
};
use crate::bonsai_db::BonsaiDb;

pub struct ContractStorageTrieView<'a>(pub(crate) BonsaiStorage<BasicId, BonsaiDb<'a>, Pedersen>);
pub struct ContractStorageTrieViewMut<'a>(pub(crate) BonsaiStorage<BasicId, BonsaiDb<'a>, Pedersen>);

impl ContractStorageTrieView<'_> {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
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

        self.0
            .get_at(identifier, &key, BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(
        &self,
        identifier: &ContractAddress,
    ) -> Result<Vec<(StorageKey, StarkFelt)>, DeoxysStorageError> {
        Ok(self
            .0
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (starkfelt(&k).try_into().unwrap(), starkfelt(&v)))
            .collect())
    }

    pub fn contains(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<bool, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
            .contains(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        self.0
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::ContractStorage))
    }
}

impl ContractStorageTrieViewMut<'_> {
    pub fn get(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<Option<Felt>, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
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

        self.0
            .get_at(identifier, &key, BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn get_storage(
        &self,
        identifier: &ContractAddress,
    ) -> Result<Vec<(StorageKey, StarkFelt)>, DeoxysStorageError> {
        Ok(self
            .0
            .get_key_value_pairs(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))?
            .into_iter()
            .map(|(k, v)| (starkfelt(&k).try_into().unwrap(), starkfelt(&v)))
            .collect())
    }

    pub fn contains(&self, identifier: &ContractAddress, key: &StorageKey) -> Result<bool, DeoxysStorageError> {
        let identifier = conv_contract_identifier(identifier);
        let key = conv_contract_storage_key(key);

        self.0
            .contains(identifier, &key)
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractStorage))
    }

    pub fn root(&self, identifier: &ContractAddress) -> Result<Felt, DeoxysStorageError> {
        self.0
            .root_hash(conv_contract_identifier(identifier))
            .map_err(|_| DeoxysStorageError::TrieRootError(TrieType::ContractStorage))
    }
    pub fn insert(
        &mut self,
        identifier: ContractAddress,
        key: StorageKey,
        value: StarkFelt,
    ) -> Result<(), DeoxysStorageError> {
        let identifier = conv_contract_identifier(&identifier);
        let key = conv_contract_storage_key(&key);
        let value = conv_contract_value(value);

        self.0
            .insert(identifier, &key, &value)
            .map_err(|_| DeoxysStorageError::StorageInsertionError(StorageType::ContractStorage))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractStorage))
    }

    pub fn init(&mut self, _identifier: &ContractAddress) -> Result<(), DeoxysStorageError> {
        // self.0
        //     .init_tree(conv_contract_identifier(identifier))
        //     .map_err(|_| DeoxysStorageError::TrieInitError(TrieType::ContractStorage))
        Ok(())
    }

    pub fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractStorage, block_number))
    }
}

fn starkfelt(bytes: &[u8]) -> StarkFelt {
    StarkFelt::new_unchecked(Felt::from_bytes_be_slice(bytes).to_bytes_be())
}
