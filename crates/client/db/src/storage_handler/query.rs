use blockifier::execution::contract_class::ContractClass;
use starknet_api::core::ContractAddress;

use super::primitives::contract_class::ContractAbi;
use super::{DeoxysStorageError, StorageView};

pub fn contract_class_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let Some(class_hash) = super::contract_class_hash().get_at(contract_address, block_number)? else {
        return Ok(None);
    };
    super::contract_class_data().get(&class_hash).map(|v| v.map(|v| v.contract_class))
}

pub fn contract_abi_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractAbi>, DeoxysStorageError> {
    let Some(class_hash) = super::contract_class_hash().get_at(contract_address, block_number)? else {
        return Ok(None);
    };

    super::contract_class_data().get(&class_hash).map(|v| v.map(|v| v.abi))
}
