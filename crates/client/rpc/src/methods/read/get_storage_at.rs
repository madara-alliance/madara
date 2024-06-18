use dp_convert::to_felt::ToFelt;
use dp_convert::to_stark_felt::ToStarkFelt;
use jsonrpsee::core::RpcResult;
use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::StarknetRpcApiError;
use crate::utils::ResultExt;
use crate::Starknet;

/// Get the value of the storage at the given address and key.
///
/// This function retrieves the value stored in a specified contract's storage, identified by a
/// contract address and a storage key, within a specified block in the current network.
///
/// ### Arguments
///
/// * `contract_address` - The address of the contract to read from. This parameter identifies the
///   contract whose storage is being queried.
/// * `key` - The key to the storage value for the given contract. This parameter specifies the
///   particular storage slot to be queried.
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter defines the state of the blockchain at which the storage value is to
///   be read.
///
/// ### Returns
///
/// Returns the value at the given key for the given contract, represented as a `Felt`.
/// If no value is found at the specified storage key, returns 0.
///
/// ### Errors
///
/// This function may return errors in the following cases:
///
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
/// * `CONTRACT_NOT_FOUND` - If the specified contract does not exist or is not deployed at the
///   given `contract_address` in the specified block.
/// * `STORAGE_KEY_NOT_FOUND` - If the specified storage key does not exist within the given
///   contract.
pub fn get_storage_at(starknet: &Starknet, contract_address: Felt, key: Felt, block_id: BlockId) -> RpcResult<Felt> {
    let block_number = starknet.get_block_n(block_id)?;

    let contract_address = contract_address.to_stark_felt().try_into().map_err(StarknetRpcApiError::from)?;
    let key = key.to_stark_felt().try_into().map_err(StarknetRpcApiError::from)?;

    // Check if the contract exists at the given address in the specified block.
    match starknet
        .backend
        .contract_class_hash()
        .is_contract_deployed_at(&contract_address, block_number)
        .or_internal_server_error("Failed to check if contract is deployed")?
    {
        true => {}
        false => return Err(StarknetRpcApiError::ContractNotFound.into()),
    }

    let value = starknet
        .backend
        .contract_storage()
        .get_at(&(contract_address, key), block_number)
        .or_internal_server_error("Failed to retrieve contract storage")?
        .unwrap_or_default();

    Ok(value.to_felt())
}
