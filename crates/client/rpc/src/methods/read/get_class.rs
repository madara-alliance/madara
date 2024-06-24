use dc_db::storage_handler::primitives::contract_class::{ContractClassWrapper, StorageContractClassData};
use dc_db::storage_handler::StorageView;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{BlockId, ContractClass, Felt};

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

pub fn get_class(starknet: &Starknet, block_id: BlockId, class_hash: Felt) -> RpcResult<ContractClass> {
    // Check if the given block exists
    starknet.get_block_info(block_id)?;

    let class = starknet
        .backend
        .contract_class_data()
        .get(&class_hash)
        .or_internal_server_error("Failed to retrieve contract class")?
        .ok_or(StarknetRpcApiError::ClassHashNotFound)?;

    let StorageContractClassData {
        contract_class,
        abi,
        sierra_program_length,
        abi_length,
        block_number: _declared_at_block,
    } = class;

    let contract_class_core: ContractClass =
        ContractClassWrapper { contract_class, abi, sierra_program_length, abi_length }
            .try_into()
            .or_else_internal_server_error(|| {
                format!("Failed to convert contract class from hash '{class_hash}' to RPC contract class")
            })?;

    Ok(contract_class_core)
}
