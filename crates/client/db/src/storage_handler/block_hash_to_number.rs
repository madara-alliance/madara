use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use mp_felt::Felt252Wrapper;
use parity_scale_codec::{Decode, Encode};

use super::{DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

pub fn insert(block_hash: Felt252Wrapper, block_number: u64) {
    DeoxysBackend::block_hash_to_number().write().unwrap().insert(&key(block_hash), &block_number.encode())
}

pub fn get(block_hash: Felt252Wrapper) -> Result<Option<u64>, DeoxysStorageError> {
    let block_number = DeoxysBackend::block_hash_to_number()
        .read()
        .unwrap()
        .get(&key(block_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?
        .map(|bytes| u64::decode(&mut &bytes[..]));

    match block_number {
        Some(Ok(block_number)) => Ok(Some(block_number)),
        Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::BlockNumber)),
        None => Ok(None),
    }
}

pub fn contains(block_hash: Felt252Wrapper) -> Result<bool, DeoxysStorageError> {
    Ok(DeoxysBackend::block_hash_to_number()
        .read()
        .unwrap()
        .contains(&key(block_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::BlockNumber))?)
}

pub fn commit(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::block_hash_to_number()
        .write()
        .unwrap()
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::BlockNumber))?)
}

pub fn revert_to(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::block_hash_to_number()
        .write()
        .unwrap()
        .revert_to(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::BlockNumber, block_number))?)
}

fn key(key: Felt252Wrapper) -> BitVec<u8, Msb0> {
    key.0.to_bytes_be().view_bits()[5..].to_owned()
}
