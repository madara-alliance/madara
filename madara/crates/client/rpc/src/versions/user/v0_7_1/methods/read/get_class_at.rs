use mp_block::BlockId;
use mp_rpc::MaybeDeprecatedContractClass;
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

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
pub fn get_class_at(
    starknet: &Starknet,
    block_id: BlockId,
    contract_address: Felt,
) -> StarknetRpcResult<MaybeDeprecatedContractClass> {
    let resolved_block_id = starknet
        .backend
        .resolve_block_id(&block_id)
        .or_internal_server_error("Error resolving block id")?
        .ok_or(StarknetRpcApiError::BlockNotFound)?;

    let class_hash = starknet
        .backend
        .get_contract_class_hash_at(&resolved_block_id, &contract_address)
        .or_internal_server_error("Error getting contract class hash at")?
        .ok_or(StarknetRpcApiError::ContractNotFound)?;

    let class_data = starknet
        .backend
        .get_class_info(&resolved_block_id, &class_hash)
        .or_internal_server_error("Error getting contract class info")?
        .ok_or_internal_server_error("Class has no info")?;

    Ok(class_data.contract_class().into())
}
