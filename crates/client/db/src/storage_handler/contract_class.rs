use blockifier::execution::contract_class::ContractClass;
use bonsai_trie::id::BasicId;
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;
use starknet_core::types::BlockId;

use super::{conv_class_key, DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

const INDENTIFIER: &[u8] = b"contract_class";

fn insert(class_hash: ClassHash, contract_class: ContractClass) {
    DeoxysBackend::contract_class().write().unwrap().insert(
        INDENTIFIER,
        &conv_class_key(&class_hash),
        &contract_class.encode(),
    );
}

fn get(class_hash: ClassHash) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let bytes = DeoxysBackend::contract_class()
        .read()
        .unwrap()
        .get(INDENTIFIER, &conv_class_key(&class_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?;

    let contract_class = match bytes {
        Some(bytes) => Some(
            ContractClass::decode(&mut &bytes[..])
                .map_err(|e| DeoxysStorageError::StorageDecodeError(StorageType::ContractClass))?,
        ),
        None => None,
    };

    Ok(contract_class)
}

fn get_at(class_hash: ClassHash, block_id: BlockId) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let contract_class_storage = DeoxysBackend::contract_class().read().unwrap();

    let change_id = match block_id {
        BlockId::Number(number) => BasicId::new(number),
        _ => contract_class_storage.get_latest_id().unwrap(),
    };

    let bytes = contract_class_storage
        .get_transactional_state(change_id, contract_class_storage.get_config())
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractClass))?
        .and_then(|transactional_state| Some(transactional_state.get(INDENTIFIER, &conv_class_key(&class_hash))));

    let contract_class = match bytes {
        Some(Ok(Some(bytes))) => Some(
            ContractClass::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractClass))?,
        ),
        _ => None,
    };

    Ok(contract_class)
}

fn commit(block_number: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_class()
        .write()
        .unwrap()
        .commit(BasicId::new(block_number + 1))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractClass))?)
}
