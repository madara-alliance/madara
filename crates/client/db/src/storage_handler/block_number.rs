use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut, StorageViewRevetible};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct BlockNumberView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

pub struct BlockNumberViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for BlockNumberView<'_> {
    type KEY = Felt252Wrapper;

    type VALUE = u64;

    fn get(self, block_hash: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let block_number = self
            .0
            .get(&conv_key(block_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?
            .map(|bytes| u64::decode(&mut &bytes[..]));

        match block_number {
            Some(Ok(block_number)) => Ok(Some(block_number)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockNumber)),
            None => Ok(None),
        }
    }

    fn contains(self, block_hash: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self
            .0
            .contains(&conv_key(block_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?)
    }
}

impl StorageViewMut for BlockNumberViewMut<'_> {
    type KEY = Felt252Wrapper;

    type VALUE = u64;

    fn insert(&mut self, block_hash: &Self::KEY, block_number: &Self::VALUE) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&conv_key(block_hash), &block_number.encode()))
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockNumber))?)
    }
}

impl StorageViewRevetible for BlockNumberViewMut<'_> {
    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockNumber, block_number))?)
    }
}

fn conv_key(key: &Felt252Wrapper) -> BitVec<u8, Msb0> {
    key.0.to_bytes_be().view_bits()[5..].to_owned()
}
