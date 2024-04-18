use bonsai_trie::id::BasicId;
use mp_contract::ContractAbi;
use parity_scale_codec::{Decode, Encode};
use starknet_api::api_core::ClassHash;
use starknet_core::types::BlockId;

use super::{conv_class_key, DeoxysStorageError, StorageType};
use crate::DeoxysBackend;

pub fn insert(class_hash: ClassHash, abi: ContractAbi) {
    DeoxysBackend::contract_abi().write().unwrap().insert(&conv_class_key(&class_hash), &abi.encode());
}

pub fn get(class_hash: ClassHash) -> Result<Option<ContractAbi>, DeoxysStorageError> {
    let contract_abi = DeoxysBackend::contract_abi()
        .read()
        .unwrap()
        .get(&conv_class_key(&class_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
        .map(|bytes| ContractAbi::decode(&mut &bytes[..]));

    match contract_abi {
        Some(Ok(contract_abi)) => Ok(Some(contract_abi)),
        Some(Err(_)) => Err(DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi)),
        None => Ok(None),
    }
}

pub fn get_at(class_hash: ClassHash, block_id: BlockId) -> Result<Option<ContractAbi>, DeoxysStorageError> {
    let contract_abi_storage = DeoxysBackend::contract_abi().read().unwrap();

    let change_id = match block_id {
        BlockId::Number(number) => BasicId::new(number),
        _ => contract_abi_storage.get_latest_id().unwrap(),
    };

    let bytes = contract_abi_storage
        .get_transactional_state(change_id, contract_abi_storage.get_config())
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?
        .and_then(|transactional_state| Some(transactional_state.get(&conv_class_key(&class_hash))));

    let contract_abi = match bytes {
        Some(Ok(Some(bytes))) => Some(
            ContractAbi::decode(&mut &bytes[..])
                .map_err(|_| DeoxysStorageError::StorageDecodeError(StorageType::ContractAbi))?,
        ),
        _ => None,
    };

    Ok(contract_abi)
}

pub fn contains(class_hash: ClassHash) -> Result<bool, DeoxysStorageError> {
    Ok(DeoxysBackend::contract_abi()
        .read()
        .unwrap()
        .contains(&conv_class_key(&class_hash))
        .map_err(|_| DeoxysStorageError::StorageRetrievalError(StorageType::ContractAbi))?)
}

pub fn commit(block_numnber: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_abi()
        .write()
        .unwrap()
        .commit(BasicId::new(block_numnber))
        .map_err(|_| DeoxysStorageError::StorageCommitError(StorageType::ContractAbi))?)
}

pub fn revert_to(block_numnber: u64) -> Result<(), DeoxysStorageError> {
    Ok(DeoxysBackend::contract_abi()
        .write()
        .unwrap()
        .revert_to(BasicId::new(block_numnber))
        .map_err(|_| DeoxysStorageError::StorageRevertError(StorageType::ContractAbi, block_numnber))?)
}
