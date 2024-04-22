use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_contract::ContractAbi;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::ClassHash;

use super::{conv_class_key, DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;

pub struct ContractAbiView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);
pub struct ContractAbiViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for ContractAbiView<'_> {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn get(self, class_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_abi = self
            .0
            .get(&conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
            .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

        match contract_abi {
            Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
            None => Ok(None),
        }
    }

    fn get_at(self, class_hash: &Self::KEY, block_number: u64) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let contract_abi = self
            .0
            .get_at(&conv_class_key(class_hash), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
            .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

        match contract_abi {
            Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
            None => Ok(None),
        }
    }

    fn contains(self, class_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        self.0
            .contains(&conv_class_key(class_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))
    }
}

impl StorageViewMut for ContractAbiViewMut<'_> {
    type KEY = ClassHash;

    type VALUE = ContractAbi;

    fn insert(&mut self, class_hash: &Self::KEY, nonce: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        self.0.insert(&conv_class_key(class_hash), &nonce.encode());
        Ok(())
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractAbi))
    }
}

impl StorageViewRevetible for ContractAbiViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        self.0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractAbi, block_number))
    }
}
