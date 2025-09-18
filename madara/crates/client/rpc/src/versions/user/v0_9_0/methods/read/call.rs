use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_rpc::v0_9_0::{BlockId, FunctionCall};
use starknet_types_core::felt::Felt;

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
pub async fn call(starknet: &Starknet, request: FunctionCall, block_id: BlockId) -> StarknetRpcResult<Vec<Felt>> {
    let view = starknet.resolve_block_view(block_id)?;

    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let FunctionCall { contract_address, entry_point_selector, calldata } = request;
    // spawn_blocking: avoid starving the tokio workers during execution.
    let results = mp_utils::spawn_blocking(move || {
        exec_context.call_contract(&contract_address, &entry_point_selector, &calldata)
    })
    .await?;

    Ok(results)
}
