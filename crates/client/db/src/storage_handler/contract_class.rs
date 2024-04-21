use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use blockifier::execution::contract_class::ContractClass;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::ClassHash;

use super::{conv_class_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;

pub struct ContractClassView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

pub struct ContractClassViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for ContractClassView<'_> {
    type KEY = ClassHash;

    type VALUE = ContractClass;

    fn get(self, contract_class: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_class = self
            .0
            .get(&conv_class_key(contract_class))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?
            .map(|bytes| ContractClass::decode(&mut &bytes[..]));

        match contract_class {
            Some(Ok(contract_class)) => Ok(Some(contract_class)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Class)),
            None => Ok(None),
        }
    }

    fn get_at(self, key: &Self::KEY, block_number: u64) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_class = self
            .0
            .get_at(&conv_class_key(key), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?
            .map(|bytes| ContractClass::decode(&mut &bytes[..]));

        match contract_class {
            Some(Ok(contract_class)) => Ok(Some(contract_class)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::Class)),
            None => Ok(None),
        }
    }

    fn contains(self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self
            .0
            .contains(&conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?)
    }
}

impl StorageViewMut for ContractClassViewMut<'_> {
    type KEY = ClassHash;

    type VALUE = ContractClass;

    fn insert(&mut self, class_hash: &Self::KEY, contract_class: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&conv_class_key(class_hash), &contract_class.encode()))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number + 1))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClass))?)
    }
}

impl StorageViewRevetible for ContractClassViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractClass, block_number))?)
    }
}
