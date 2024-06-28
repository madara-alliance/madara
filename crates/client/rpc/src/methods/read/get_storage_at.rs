use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
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
pub fn get_storage_at(
    starknet: &Starknet,
    contract_address: Felt,
    key: Felt,
    block_id: BlockId,
) -> StarknetRpcResult<Felt> {
    // Check if contract exists
    starknet
        .backend
        .get_contract_class_hash_at(&block_id, &contract_address) // TODO: contains api without deser
        .or_internal_server_error("Failed to check if contract is deployed")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    let storage = starknet
        .backend
        .get_contract_storage_at(&block_id, &contract_address, &key)
        .or_internal_server_error("Error getting contract class hash at")?
        .unwrap_or(Felt::ZERO);

    Ok(storage)
}
