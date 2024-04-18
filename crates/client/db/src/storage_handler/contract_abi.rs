use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_contract::ContractAbi;
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;
use starknet_core::types::BlockId;

use super::{conv_class_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct ContractAbiView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

pub struct ContractAbiViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for ContractAbiView<'_> {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn get(self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_abi = self
            .0
            .get(&conv_class_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
            .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

        match contract_abi {
            Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
            None => Ok(None),
        }
    }

    fn contains(self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self
            .0
            .contains(&conv_class_key(&key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?)
    }
}

impl StorageViewMut for ContractAbiViewMut<'_> {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn insert(&mut self, key: &Self::KEY, value: &Self::VALUE) {
        self.0.insert(&conv_class_key(key), &value.encode());
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractAbi))?)
    }

    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractAbi, block_number))?)
    }
}
