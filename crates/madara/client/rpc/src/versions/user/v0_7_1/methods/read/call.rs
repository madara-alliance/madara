use std::sync::Arc;

use mc_exec::ExecutionContext;
use mp_block::BlockId;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::FunctionCall;

use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::versions::user::v0_7_1::methods::trace::trace_transaction::EXECUTION_UNSUPPORTED_BELOW_VERSION;
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
pub fn call(starknet: &Starknet, request: FunctionCall<Felt>, block_id: BlockId) -> StarknetRpcResult<Vec<Felt>> {
    let block_info = starknet.get_block_info(&block_id)?;

    let exec_context = ExecutionContext::new_at_block_end(Arc::clone(&starknet.backend), &block_info)?;

    if block_info.protocol_version() < &EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let FunctionCall { contract_address, entry_point_selector, calldata } = request;
    let results = exec_context.call_contract(&contract_address, &entry_point_selector, &calldata)?;

    Ok(results)
}
