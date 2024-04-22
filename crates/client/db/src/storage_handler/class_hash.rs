use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use parity_scale_codec::{Decode, Encode};
use starknet_api::core::{ClassHash, ContractAddress};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct ClassHashView;

pub struct ClassHashViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for ClassHashView {
    type KEY = ContractAddress;

    type VALUE = ClassHash;

    fn get(self, contract_address: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let class_hash = class_hash_db()
            .get(&conv_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ClassHash))?
            .map(|bytes| ClassHash::decode(&mut &bytes[..]));

        match class_hash {
            Some(Ok(class_hash)) => Ok(Some(class_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ClassHash)),
            None => Ok(None),
        }
    }

    fn get_at(
        self,
        contract_address: &Self::KEY,
        block_number: u64,
    ) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let class_hash = class_hash_db()
            .get_at(&conv_key(contract_address), BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ClassHash))?
            .map(|bytes| ClassHash::decode(&mut &bytes[..]));

        match class_hash {
            Some(Ok(class_hash)) => Ok(Some(class_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ClassHash)),
            None => Ok(None),
        }
    }

    fn contains(self, contract_address: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(class_hash_db()
            .contains(&conv_key(contract_address))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ClassHash))?)
    }
}

impl StorageViewMut for ClassHashViewMut<'_> {
    type KEY = ContractAddress;

    type VALUE = ClassHash;

    fn insert(&mut self, key: &Self::KEY, value: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&conv_key(key), &value.encode()))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ClassHash))?)
    }
}

impl StorageViewRevetible for ClassHashViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ClassHash, block_number))?)
    }
}

fn conv_key(key: &ContractAddress) -> BitVec<u8, Msb0> {
    key.0.0.0.view_bits()[5..].to_owned()
}

fn class_hash_db<'a>() -> RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>> {
    DeoxysBackend::class_hash().read().unwrap()
}
