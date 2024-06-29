use starknet_core::types::BlockId;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Get the contract class hash in the given block for the contract deployed at the given
/// address
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag
/// * `contract_address` - The address of the contract whose class hash will be returned
///
/// ### Returns
///
/// * `class_hash` - The class hash of the given contract
pub fn get_class_hash_at(starknet: &Starknet, block_id: BlockId, contract_address: Felt) -> StarknetRpcResult<Felt> {
    let block_number = starknet.get_block_n(block_id)?;

    let class_hash = starknet
        .backend
        .contract_class_hash()
        .get_at(&contract_address, block_number)
        .or_internal_server_error("Failed to retrieve contract class hash")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    Ok(class_hash)
}
