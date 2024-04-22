use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct BlockNumberView;

pub struct BlockNumberViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl BlockNumberView {
    pub fn get(self, block_hash: &Felt252Wrapper) -> Result<Option<u64>, DeoxysStorageError> {
        let block_number = block_number_db()
            .get(&key(block_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?
            .map(|bytes| u64::decode(&mut &bytes[..]));

        match block_number {
            Some(Ok(block_number)) => Ok(Some(block_number)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockNumber)),
            None => Ok(None),
        }
    }

    pub fn contains(self, block_hash: &Felt252Wrapper) -> Result<bool, DeoxysStorageError> {
        Ok(block_number_db()
            .contains(&key(block_hash))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?)
    }
}

impl BlockNumberViewMut<'_> {
    pub fn insert(&mut self, block_hash: &Felt252Wrapper, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&key(block_hash), &block_number.encode()))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockNumber))?)
    }

    pub fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockNumber, block_number))?)
    }
}

fn key(key: &Felt252Wrapper) -> BitVec<u8, Msb0> {
    key.0.to_bytes_be().view_bits()[5..].to_owned()
}

fn block_number_db<'a>() -> RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>> {
    DeoxysBackend::block_number().read().unwrap()
}
