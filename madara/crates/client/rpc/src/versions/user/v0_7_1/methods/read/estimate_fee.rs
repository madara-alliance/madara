use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::tx_api_to_blockifier;
use crate::Starknet;
use blockifier::transaction::account_transaction::ExecutionFlags;
use mc_exec::execution::TxInfo;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_convert::ToFelt;
use mp_rpc::v0_7_1::{BlockId, BroadcastedTxn, FeeEstimate, SimulationFlagForEstimateFee};
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
            let execution_flags = ExecutionFlags { only_query, charge_fee: false, validate, strict_nonce_check: validate };
            Ok(tx_api_to_blockifier(api_tx, execution_flags)?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()?;

    let tips = transactions.iter().map(|tx| tx.tip().unwrap_or_default()).collect::<Vec<_>>();

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions_for_estimation([], transactions)?, exec_context))
    })
    .await
    .map_err(StarknetRpcApiError::from_exec_error_v0_7)?;

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
                        .map(|e| e.to_string())
                        .unwrap_or_default()
                        .into(),
                });
            }
            Ok(exec_context.execution_result_to_fee_estimate_v0_7(result, tip)?)
        })
        .collect::<Result<_, _>>()?;

    Ok(fee_estimates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::rpc_test_setup_with_execution;
    use assert_matches::assert_matches;
    use mc_devnet::{Call, Multicall, Selector};
    use mp_rpc::v0_7_1::{BlockTag, BroadcastedInvokeTxn, DaMode, InvokeTxnV3, ResourceBounds, ResourceBoundsMapping};
    use starknet_types_core::felt::Felt;

    /// v0.7.1 predates structured execution errors: a hard execution failure must surface its
    /// TRANSACTION_EXECUTION_ERROR `execution_error` data as a flat string, not as the structured
    /// object used by v0.8+.
    #[tokio::test]
    async fn estimate_fee_execution_error_is_flat_string() {
        let (backend, rpc, keys) = rpc_test_setup_with_execution().await;
        let account = &keys.0[0];

        // Invalid signature with validation enabled: a hard (non-revert) execution error.
        let tx = BroadcastedTxn::Invoke(BroadcastedInvokeTxn::V3(InvokeTxnV3 {
            sender_address: account.address,
            calldata: Multicall::default()
                .with(Call {
                    to: backend.chain_config().native_fee_token_address.to_felt(),
                    selector: Selector::from("transfer"),
                    calldata: vec![account.address, Felt::ONE, Felt::ZERO],
                })
                .flatten()
                .collect::<Vec<_>>()
                .into(),
            signature: vec![Felt::ONE, Felt::TWO].into(),
            nonce: Felt::ZERO,
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 6000000000, max_price_per_unit: 100000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        }));
        let result = estimate_fee(&rpc, vec![tx], vec![], BlockId::Tag(BlockTag::Latest)).await;

        assert_matches!(result.unwrap_err(), StarknetRpcApiError::TxnExecutionError { tx_index: 0, error } => {
            assert!(error.is_string(), "v0.7.1 execution_error must be a flat string, got: {error}");
        });
    }
}
