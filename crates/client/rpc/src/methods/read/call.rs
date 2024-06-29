use dc_exec::{block_context, call_contract};
use starknet_core::types::{BlockId, FunctionCall};
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::methods::trace::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
use crate::Starknet;

/// Call a Function in a Contract Without Creating a Transaction
///
/// ### Arguments
///
/// * `request` - The details of the function call to be made. This includes information such as the
///   contract address, function signature, and arguments.
/// * `block_id` - The identifier of the block used to reference the state or call the transaction
///   on. This can be the hash of the block, its number (height), or a specific block tag.
///
/// ### Returns
///
/// * `result` - The function's return value, as defined in the Cairo output. This is an array of
///   field elements (`Felt`).
///
/// ### Errors
///
/// This method may return the following errors:
/// * `CONTRACT_NOT_FOUND` - If the specified contract address does not exist.
/// * `CONTRACT_ERROR` - If there is an error with the contract or the function call.
/// * `BLOCK_NOT_FOUND` - If the specified block does not exist in the blockchain.
pub fn call(starknet: &Starknet, request: FunctionCall, block_id: BlockId) -> StarknetRpcResult<Vec<Felt>> {
    let block_info = starknet.get_block_info(block_id).unwrap();
    // let block_info = starknet.get_block_info(block_id)?;

    if block_info.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let block_context = block_context(block_info.header(), &starknet.chain_id());

    let FunctionCall { contract_address, entry_point_selector, calldata } = request;

    Ok(call_contract(starknet.clone_backend(), &contract_address, &entry_point_selector, &calldata, &block_context)?)
}
