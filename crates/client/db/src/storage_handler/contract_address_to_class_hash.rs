use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use bonsai_trie::id::BasicId;
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::{ClassHash, ContractAddress};

use super::{DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

pub fn insert(contract_address: &ContractAddress, class_hash: &ClassHash) {
    DeoxysBackend::contract_address_to_class_hash()
        .write()
        .unwrap()
        .insert(&key(contract_address), &class_hash.encode())
}

pub fn get(contract_address: &ContractAddress) -> Result<Option<ClassHash>, DeoxysStorageError> {
    let class_hash = DeoxysBackend::contract_address_to_class_hash()
        .read()
        .unwrap()
        .get(&key(contract_address))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ClassHash))?
        .map(|bytes| ClassHash::decode(&mut &bytes[..]));

    match class_hash {
        Some(Ok(class_hash)) => Ok(Some(class_hash)),
        Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ClassHash)),
        None => Ok(None),
    }
}

pub fn contains(contract_address: &ContractAddress) -> Result<bool, DeoxysStorageError> {
    Ok(DeoxysBackend::contract_address_to_class_hash()
        .read()
        .unwrap()
        .contains(&key(contract_address))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ClassHash))?)
}

pub fn commit(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_address_to_class_hash()
        .write()
        .unwrap()
        .commit(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ClassHash))?)
}

pub fn revert_to(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_address_to_class_hash()
        .write()
        .unwrap()
        .revert_to(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ClassHash, block_number))?)
}

fn key(key: &ContractAddress) -> BitVec<u8, Msb0> {
    key.0.0.0.view_bits()[5..].to_owned()
}
