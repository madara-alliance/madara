use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use anyhow::Context;
use mp_block::BlockId;
use mp_rpc::v0_7_1::MaybeDeprecatedContractClass;
use starknet_types_core::felt::Felt;

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
    let view = starknet.backend.view_on(block_id)?;
    let class_hash =
        view.get_contract_class_hash(&contract_address)?.ok_or(StarknetRpcApiError::contract_not_found())?;
    let class_info = view.get_class_info(&class_hash)?.context("Class info should exist")?;

    Ok(class_info.contract_class().into())
}
