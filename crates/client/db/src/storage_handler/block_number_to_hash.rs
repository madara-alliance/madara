use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

pub fn insert(block_number: u64, block_hash: Felt252Wrapper) {
    DeoxysBackend::block_hash_to_number().write().unwrap().insert(&key(block_number), &block_hash.encode())
}

pub fn get(block_number: u64) -> Result<Option<Felt252Wrapper>, DeoxysStorageError> {
    let block_hash = DeoxysBackend::block_number_to_hash()
        .read()
        .unwrap()
        .get(&key(block_number))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?
        .map(|bytes| Felt252Wrapper::decode(&mut &bytes[..]));

    match block_hash {
        Some(Ok(block_hash)) => Ok(Some(block_hash)),
        Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockHash)),
        None => Ok(None),
    }
}

pub fn contains(block_number: u64) -> Result<bool, DeoxysStorageError> {
    Ok(DeoxysBackend::block_number_to_hash()
        .read()
        .unwrap()
        .contains(&key(block_number))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockHash))?)
}

pub fn commit(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::block_number_to_hash()
        .write()
        .unwrap()
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockHash))?)
}

pub fn revert_to(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::block_number_to_hash()
        .write()
        .unwrap()
        .revert_to(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockHash, block_number))?)
}

fn key(key: u64) -> BitVec<u8, Msb0> {
    key.to_be_bytes().view_bits()[5..].to_owned()
}
