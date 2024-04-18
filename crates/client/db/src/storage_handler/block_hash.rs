use std::sync::{RwLockReadGuard, RwLockWriteGuard};

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use bonsai_trie::RevertibleStorage;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType, StorageView, StorageViewMut};
use crate::bonsai_db::BonsaiDb;
use crate::DeoxysBackend;

pub struct BlockHashView<'a>(pub(crate) RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

pub struct BlockHashViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl StorageView for BlockHashView<'_> {
    type KEY = u64;

    type VALUE = Felt252Wrapper;

    fn get(self, block_number: &Self::KEY) -> Result<Option<Self::VALUE>, DeoxysStorageError> {
        let block_hash = self
            .0
            .get(&key(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?
            .map(|bytes| Felt252Wrapper::decode(&mut &bytes[..]));

        match block_hash {
            Some(Ok(block_hash)) => Ok(Some(block_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockHash)),
            None => Ok(None),
        }
    }

    fn contains(self, block_number: &Self::KEY) -> Result<bool, DeoxysStorageError> {
        Ok(self
            .0
            .contains(&key(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?)
    }
}

impl StorageViewMut for BlockHashViewMut<'_> {
    type KEY = u64;

    type VALUE = Felt252Wrapper;

    fn insert(&mut self, block_number: &Self::KEY, block_hash: &Self::VALUE) {
        self.0.insert(&key(block_number), &block_hash.encode())
    }

    fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockHash))?)
    }

    fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockHash, block_number))?)
    }
}

fn key(key: &u64) -> BitVec<u8, Msb0> {
    key.to_be_bytes().view_bits()[5..].to_owned()
}
