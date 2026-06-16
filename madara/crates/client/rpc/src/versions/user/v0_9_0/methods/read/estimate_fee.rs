use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::tx_api_to_blockifier;
use crate::Starknet;
use blockifier::transaction::account_transaction::ExecutionFlags;
use mc_exec::execution::TxInfo;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_9_0::{BlockId, BroadcastedTxn, FeeEstimate, SimulationFlagForEstimateFee};
use mp_transactions::{IntoStarknetApiExt, ToBlockifierError};

/// Estimate the fee associated with transaction
///
/// # Arguments
///
/// * `request` - starknet transaction request
/// * `block_id` - hash of the requested block, number (height), or tag
///
/// # Returns
///
/// * `fee_estimate` - fee estimate in gwei
pub async fn estimate_fee(
    starknet: &Starknet,
    request: Vec<BroadcastedTxn>,
    simulation_flags: Vec<SimulationFlagForEstimateFee>,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<FeeEstimate>> {
    tracing::debug!("estimate fee on block_id {block_id:?}");
    crate::utils::check_estimate_batch_size(request.len(), "estimated")?;
    let view = starknet.resolve_block_view(block_id)?;
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let validate = !simulation_flags.contains(&SimulationFlagForEstimateFee::SkipValidate);

    let transactions = request
        .into_iter()
        .map(|tx| {
            let only_query = tx.is_query();
            let (api_tx, _) =
                tx.into_starknet_api(view.backend().chain_config().chain_id.to_felt(), exec_context.protocol_version)?;
            let execution_flags =
                ExecutionFlags { only_query, charge_fee: false, validate, strict_nonce_check: validate };
            Ok(tx_api_to_blockifier(api_tx, execution_flags)?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()?;

    let tips = transactions.iter().map(|tx| tx.tip().unwrap_or_default()).collect::<Vec<_>>();

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions_for_estimation([], transactions)?, exec_context))
    })
    .await?;

    let fee_estimates = execution_results
        .iter()
        .zip(tips)
        .enumerate()
        .map(|(index, (result, tip))| {
            if result.execution_info.is_reverted() {
                return Err(StarknetRpcApiError::TxnExecutionError {
                    tx_index: index,
                    error: result
                        .execution_info
                        .revert_error
                        .as_ref()
                        .map(crate::utils::contract_execution_error_from_revert)
                        // Reverted executions always carry a revert_error; make the fallback
                        // visible instead of silently emitting null.
                        .unwrap_or_else(|| serde_json::json!("unknown revert reason")),
                });
            }
            Ok(FeeEstimate {
                common: exec_context.execution_result_to_fee_estimate_v0_9(result, tip)?,
                unit: mp_rpc::v0_9_0::PriceUnitFri::Fri,
            })
        })
        .collect::<Result<_, _>>()?;

    Ok(fee_estimates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{devnet_transfer_tx, rpc_test_setup_with_execution};
    use assert_matches::assert_matches;
    use mp_rpc::v0_9_0::BlockTag;
    use starknet_types_core::felt::Felt;

    /// The error 41 data must blame the actual failing transaction, not transaction 0: the first
    /// transaction is valid, the second fails validation (bad signature).
    #[tokio::test]
    async fn estimate_fee_reports_failing_transaction_index() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let txs = vec![
            devnet_transfer_tx(&backend, &keys.0[0], Felt::ZERO, true),
            devnet_transfer_tx(&backend, &keys.0[1], Felt::ZERO, false),
        ];
        let result = estimate_fee(&rpc, txs, vec![], BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::TxnExecutionError { tx_index: 1, error } => {
            // The validation failure is reported as a structured CONTRACT_EXECUTION_ERROR rooted
            // at the failing account contract.
            assert_eq!(error["contract_address"], serde_json::json!(keys.0[1].address));
        });
    }

    #[tokio::test]
    async fn estimate_fee_success() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let txs = vec![devnet_transfer_tx(&backend, &keys.0[0], Felt::ZERO, true)];
        let estimates = estimate_fee(&rpc, txs, vec![], BlockId::Tag(BlockTag::Latest)).await.unwrap();

        assert_eq!(estimates.len(), 1);
        assert!(estimates[0].common.overall_fee > 0);
    }

    /// SKIP_VALIDATE must relax the strict nonce check, like pathfinder
    /// (`strict_nonce_check: !skip_validate`) and juno: wallets estimate queued transactions with
    /// future nonces. The signature is valid so the future nonce is the only relaxed check.
    #[tokio::test]
    async fn estimate_fee_skip_validate_allows_future_nonce() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let txs = vec![devnet_transfer_tx(&backend, &keys.0[0], Felt::from(5), true)];
        let estimates =
            estimate_fee(&rpc, txs, vec![SimulationFlagForEstimateFee::SkipValidate], BlockId::Tag(BlockTag::Latest))
                .await
                .unwrap();

        assert_eq!(estimates.len(), 1);
    }

    /// Estimation executes each transaction (several times with L2 gas discovery): the
    /// per-request transaction count must be bounded.
    #[tokio::test]
    async fn estimate_fee_rejects_oversized_batch() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let tx = devnet_transfer_tx(&backend, &keys.0[0], Felt::ZERO, false);
        let txs = vec![tx; crate::constants::MAX_ESTIMATE_TRANSACTIONS + 1];
        let result = estimate_fee(&rpc, txs, vec![], BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::InvalidParams { .. });
    }

    /// Without SKIP_VALIDATE the strict nonce check still applies.
    #[tokio::test]
    async fn estimate_fee_future_nonce_fails_without_skip_validate() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;

        let txs = vec![devnet_transfer_tx(&backend, &keys.0[0], Felt::from(5), true)];
        let result = estimate_fee(&rpc, txs, vec![], BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::TxnExecutionError { tx_index: 0, .. });
    }
}
