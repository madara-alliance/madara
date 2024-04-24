use jsonrpsee::core::RpcResult;
use mc_db::storage_handler::query::{contract_abi_by_address, contract_class_by_address};
use mp_contract::class::ContractClassWrapper;
use starknet_api::core::{ContractAddress, PatriciaKey};
use starknet_api::hash::StarkFelt;
use starknet_core::types::{BlockId, ContractClass, FieldElement};

use crate::errors::StarknetRpcApiError;
use crate::methods::trace::utils::block_number_by_id;

/// Get the Contract Class Definition at a Given Address in a Specific Block
///
/// ### Arguments
///
/// * `block_id` - The identifier of the block. This can be the hash of the block, its number
///   (height), or a specific block tag.
/// * `contract_address` - The address of the contract whose class definition will be returned.
///
/// ### Returns
///
/// * `contract_class` - The contract class definition. This may be either a standard contract class
///   or a deprecated contract class, depending on the contract's status and the blockchain's
///   version.
///
/// ### Errors
///
/// This method may return the following errors:
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
/// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
pub fn get_class_at(block_id: BlockId, contract_address: FieldElement) -> RpcResult<ContractClass> {
    let block_number = block_number_by_id(block_id);
    let key = ContractAddress(PatriciaKey(StarkFelt(contract_address.to_bytes_be())));

    let Ok(Some(contract_class)) = contract_class_by_address(&key, block_number) else {
        log::error!("Failed to retrieve contract class at '{contract_address}'");
        return Err(StarknetRpcApiError::ContractNotFound.into());
    };

    // Blockifier classes do not store ABI, has to be retrieved separately
    let Ok(Some(contract_abi)) = contract_abi_by_address(&key, block_number) else {
        log::error!("Failed to retrieve contract ABI at '{contract_address}'");
        return Err(StarknetRpcApiError::ContractNotFound.into());
    };

    // converting from stored Blockifier class to rpc class
    Ok(ContractClassWrapper { contract: contract_class, abi: contract_abi }.try_into().map_err(|e| {
        log::error!("Failed to convert contract class at address '{contract_address}' to RPC contract class: {e}");
        StarknetRpcApiError::InternalServerError
    })?)
}
