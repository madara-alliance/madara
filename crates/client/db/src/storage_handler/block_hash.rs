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

pub struct BlockHashView;

pub struct BlockHashViewMut<'a>(pub(crate) RwLockWriteGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>>);

impl BlockHashView {
    pub fn get(self, block_number: u64) -> Result<Option<Felt252Wrapper>, DeoxysStorageError> {
        let block_hash = block_hash_db()
            .get(&key(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?
            .map(|bytes| Felt252Wrapper::decode(&mut &bytes[..]));

        match block_hash {
            Some(Ok(block_hash)) => Ok(Some(block_hash)),
            Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockHash)),
            None => Ok(None),
        }
    }

    pub fn contains(self, block_number: u64) -> Result<bool, DeoxysStorageError> {
        Ok(block_hash_db()
            .contains(&key(block_number))
            .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?)
    }
}

impl BlockHashViewMut<'_> {
    pub fn insert(&mut self, block_number: u64, block_hash: &Felt252Wrapper) -> Result<(), DeoxysStorageError> {
        Ok(self.0.insert(&key(block_number), &block_hash.encode()))
    }

    pub fn commit(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .commit(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockHash))?)
    }

    pub fn revert_to(&mut self, block_number: u64) -> Result<(), DeoxysStorageError> {
        Ok(self
            .0
            .revert_to(BasicId::new(block_number))
            .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockHash, block_number))?)
    }
}

fn key(key: u64) -> BitVec<u8, Msb0> {
    key.to_be_bytes().view_bits()[5..].to_owned()
}

fn block_hash_db<'a>() -> RwLockReadGuard<'a, RevertibleStorage<BasicId, BonsaiDb<'static>>> {
    DeoxysBackend::block_hash().read().unwrap()
}
