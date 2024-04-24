use blockifier::execution::contract_class::ContractClass;
use mp_contract::ContractAbi;
use starknet_api::core::ContractAddress;

use super::{DeoxysStorageError, StorageView};

pub fn contract_class_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractClass>, DeoxysStorageError> {
    let Some(contract_data) = super::contract_data().get_at(contract_address, block_number)? else {
        return Ok(None);
    };
    super::contract_class_data().get(&contract_data.class_hash).map(|v| v.map(|v| v.contract_class))
}

pub fn contract_abi_by_address(
    contract_address: &ContractAddress,
    block_number: u64,
) -> Result<Option<ContractAbi>, DeoxysStorageError> {
    let Some(contract_data) = super::contract_data().get_at(contract_address, block_number)? else {
        return Ok(None);
    };

    super::contract_class_data().get(&contract_data.class_hash).map(|v| v.map(|v| v.abi))
}
