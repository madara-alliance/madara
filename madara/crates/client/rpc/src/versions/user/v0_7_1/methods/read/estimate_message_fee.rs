use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::execute_message_fee_estimation;
use crate::Starknet;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_7_1::{BlockId, FeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;

/// Estimate the L2 fee of a message sent on L1
///
/// # Arguments
///
/// * `message` - the message to estimate
/// * `block_id` - hash, number (height), or tag of the requested block
///
/// # Returns
///
/// * `FeeEstimate` - the fee estimation (gas consumed, gas price, overall fee, unit)
///
/// # Errors
///
/// BlockNotFound : If the specified block does not exist.
/// ContractNotFound : If the specified contract address does not exist.
/// ContractError : If there is an error with the contract.
pub async fn estimate_message_fee(
    starknet: &Starknet,
    message: MsgFromL1,
    block_id: BlockId,
) -> StarknetRpcResult<FeeEstimate> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    let view = starknet.resolve_block_view(block_id)?;
    let exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let l1_handler: L1HandlerTransaction = message.into();
    let chain_id = view.backend().chain_config().chain_id.to_felt();
    let (execution_result, exec_context, tip) =
        execute_message_fee_estimation(exec_context, l1_handler, chain_id).await?;

    // A failed L1 handler execution is not an error for blockifier: it returns a successful
    // execution with `revert_error` set. Surface it as CONTRACT_ERROR instead of returning a fee
    // estimate for a message that cannot be executed.
    if let Some(revert_error) = &execution_result.execution_info.revert_error {
        return Err(StarknetRpcApiError::ContractError { revert_error: revert_error.to_string().into() });
    }

    let fee_estimate = exec_context.execution_result_to_fee_estimate_v0_7(&execution_result, tip)?;

    Ok(fee_estimate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_rpc::v0_7_1::BlockTag;

    crate::test_utils::estimate_message_fee_tests!(
        |estimate| estimate.overall_fee != ::starknet_types_core::felt::Felt::ZERO
    );
}
