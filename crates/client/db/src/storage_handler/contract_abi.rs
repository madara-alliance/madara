use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_contract::ContractAbi;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::ClassHash;

use super::{conv_class_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct ContractAbiView;

pub struct ContractAbiViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for ContractAbiView {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn get(self, key: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_abi = contract_abi_db()
            .get(&conv_class_key(key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
            .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

        match contract_abi {
            Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
            None => Ok(None),
        }
    }

    fn get_at(self, key: &Self::KEY, block_number: u64) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_abi = contract_abi_db()
            .get_at(&conv_class_key(key), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
            .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

        match contract_abi {
            Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
            None => Ok(None),
        }
    }

    fn contains(self, key: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(contract_abi_db()
            .contains(&conv_class_key(&key))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?)
    }
}

impl StorageViewMut for ContractAbiViewMut<'_> {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn insert(&mut self, key: &Self::KEY, value: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&conv_class_key(key), &value.encode()))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractAbi))?)
    }
}

impl StorageViewRevetible for ContractAbiViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractAbi, block_number))?)
    }
}

fn contract_abi_db<'a>() -> RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>> {
    DeoxysBackend::contract_abi().read().unwrap()
}
