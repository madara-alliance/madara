use blockifier::execution::contract_class::ContractClass;
use bonsai_trie::id::BasicId;
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;
use starknet_core::types::BlockId;

use super::{conv_class_key, DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

pub fn insert(class_hash: ClassHash, contract_class: ContractClass) {
    DeoxysBackend::contract_class().write().unwrap().insert(&conv_class_key(&class_hash), &contract_class.encode());
}

pub fn get(class_hash: ClassHash) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let bytes = DeoxysBackend::contract_class()
        .read()
        .unwrap()
        .get(&conv_class_key(&class_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?;

    let contract_class = match bytes {
        Some(bytes) => Some(
            ContractClass::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractClass))?,
        ),
        None => None,
    };

    Ok(contract_class)
}

pub fn get_at(class_hash: ClassHash, block_id: BlockId) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let contract_class_storage = DeoxysBackend::contract_class().read().unwrap();

    let change_id = match block_id {
        BlockId::Number(number) => BasicId::new(number),
        _ => contract_class_storage.get_latest_id().unwrap(),
    };

    let bytes = contract_class_storage
        .get_transactional_state(change_id, contract_class_storage.get_config())
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?
        .and_then(|transactional_state| Some(transactional_state.get(&conv_class_key(&class_hash))));

    let contract_class = match bytes {
        Some(Ok(Some(bytes))) => Some(
            ContractClass::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractClass))?,
        ),
        _ => None,
    };

    Ok(contract_class)
}

pub fn contains(class_hash: ClassHash) -> Result<bool, DeoxysStorageError> {
    Ok(DeoxysBackend::contract_class()
        .read()
        .unwrap()
        .contains(&conv_class_key(&class_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?)
}

pub fn commit(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_class()
        .write()
        .unwrap()
        .commit(BasicId::new(block_number + 1))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClass))?)
}

pub fn revert_to(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_class()
        .write()
        .unwrap()
        .revert_to(BasicId::new(block_number))
        .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractClass, block_number))?)
}
