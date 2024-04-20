use blockifier::execution::contract_class::ContractClass;
use mp_contract::ContractAbi;
use starknet_api::api_core::ContractAddress;

use super::{DeoxysStorageError, StorageView};

pub fn contract_class_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let Some(class_hash) = super::class_hash()?.get_at(contract_address, block_number)? else {
        return Ok(None);
    };
    super::contract_class()?.get_at(&class_hash, block_number)
}

pub fn contract_abi_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractAbi>, DeoxysStorageError> {
    let Some(class_hash) = super::class_hash()?.get_at(contract_address, block_number)? else {
        return Ok(None);
    };

    super::contract_abi()?.get_at(&class_hash, block_number)
}
