use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::Starknet;
use anyhow::Context;
use mc_exec::execution::TxInfo;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_7_1::{BlockId, FeeEstimate, MsgFromL1};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::{fields::Fee, TransactionHash};

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
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let l1_handler: L1HandlerTransaction = message.into();
    let tx_hash = l1_handler.compute_hash(
        view.backend().chain_config().chain_id.to_felt(),
        /* offset_version */ false,
        /* legacy */ false,
    );
    let tx: starknet_api::transaction::L1HandlerTransaction = l1_handler.try_into().unwrap();
    let transaction = blockifier::transaction::transaction_execution::Transaction::L1Handler(
        starknet_api::executable_transaction::L1HandlerTransaction {
            tx,
            tx_hash: TransactionHash(tx_hash),
            // Blockifier rejects successfully-executed L1 handlers whose paid fee is zero. The
            // amount paid on L1 has no effect on the estimate itself, so use 1 like the other
            // implementations do.
            paid_fee_on_l1: Fee(1),
        },
    );

    let tip = transaction.tip().unwrap_or_default();
    // spawn_blocking: avoid starving the tokio workers during execution.
    let (mut execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], [transaction])?, exec_context))
    })
    .await?;

    let execution_result = execution_results.pop().context("There should be one result")?;

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
    use crate::test_utils::{rpc_test_setup_with_execution, TEST_CONTRACT_ADDRESS};
    use assert_matches::assert_matches;
    use mp_convert::ToFelt as _;
    use mp_rpc::v0_7_1::BlockTag;
    use starknet_core::utils::get_selector_from_name;
    use starknet_types_core::felt::Felt;

    const FROM_L1_ADDRESS: &str = "0x000000000000000000000000000000000000beef";

    /// A message whose handler executes successfully must return a fee estimate. Regression test:
    /// the L1 handler used to be built with `paid_fee_on_l1: 0`, which blockifier rejects on the
    /// *success* path, so every valid message errored.
    #[tokio::test]
    async fn estimate_message_fee_success() {
        let (_backend, rpc, _keys) = rpc_test_setup_with_execution().await;

        let message = MsgFromL1 {
            from_address: FROM_L1_ADDRESS.to_string(),
            to_address: TEST_CONTRACT_ADDRESS,
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            payload: vec![Felt::ONE, Felt::TWO],
        };
        let estimate = estimate_message_fee(&rpc, message, BlockId::Tag(BlockTag::Latest)).await.unwrap();

        assert!(estimate.overall_fee != Felt::ZERO);
    }

    /// A message whose handler fails must surface as CONTRACT_ERROR. Regression test: blockifier
    /// reports a failed L1 handler as a successful execution with `revert_error` set, which used
    /// to be converted into a fee estimate.
    #[tokio::test]
    async fn estimate_message_fee_reverted_returns_contract_error() {
        let (backend, rpc, _keys) = rpc_test_setup_with_execution().await;

        // The fee token ERC20 has no l1 handler: executing the message fails.
        let message = MsgFromL1 {
            from_address: FROM_L1_ADDRESS.to_string(),
            to_address: backend.chain_config().native_fee_token_address.to_felt(),
            entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
            payload: vec![Felt::ONE, Felt::TWO],
        };
        let result = estimate_message_fee(&rpc, message, BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::ContractError { .. });
    }
}
