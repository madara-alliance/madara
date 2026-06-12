use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::execute_message_fee_estimation;
use crate::Starknet;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_10_0::{BlockId, MessageFeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;

/// Estimates the L2 fee for an L1 message.
///
/// v0.10.0: Returns `CONTRACT_NOT_FOUND` if the L1 handler contract (`to_address`) doesn't exist on L2.
pub async fn estimate_message_fee(
    starknet: &Starknet,
    message: MsgFromL1,
    block_id: BlockId,
) -> StarknetRpcResult<MessageFeeEstimate> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    let view = starknet.resolve_block_view(block_id)?;
    let exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = view.state_view();
    if state_view.get_contract_class_hash(&message.to_address)?.is_none() {
        return Err(StarknetRpcApiError::ContractNotFound {
            error: format!("Contract at address {:#x} not found", message.to_address).into(),
        });
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

    let fee_estimate = exec_context.execution_result_to_fee_estimate_v0_9(&execution_result, tip)?;

    Ok(MessageFeeEstimate { common: fee_estimate, unit: mp_rpc::v0_10_0::PriceUnitWei::Wei })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mp_rpc::v0_10_0::BlockTag;

    crate::test_utils::estimate_message_fee_tests!(|estimate| estimate.common.overall_fee > 0);
}
