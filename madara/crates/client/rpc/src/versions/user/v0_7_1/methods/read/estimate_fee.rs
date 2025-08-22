use crate::errors::StarknetRpcApiError;
use crate::errors::StarknetRpcResult;
use crate::utils::tx_api_to_blockifier;
use crate::Starknet;
use anyhow::Context;
use blockifier::transaction::account_transaction::ExecutionFlags;
use mc_exec::MadaraBlockViewExecutionExt;
use mc_exec::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use mp_block::BlockId;
use mp_convert::ToFelt;
use mp_rpc::{BroadcastedTxn, FeeEstimate, SimulationFlagForEstimateFee};
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
    let view = starknet.backend.block_view(block_id)?;
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
            let execution_flags = ExecutionFlags { only_query, charge_fee: false, validate, strict_nonce_check: true };
            Ok(tx_api_to_blockifier(api_tx, execution_flags)?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()
        .context("Failed to convert BroadcastedTransaction to AccountTransaction")?;

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], transactions)?, exec_context))
    })
    .await?;

    let fee_estimates = execution_results.iter().enumerate().try_fold(
        Vec::with_capacity(execution_results.len()),
        |mut acc, (index, result)| {
            if result.execution_info.is_reverted() {
                return Err(StarknetRpcApiError::TxnExecutionError {
                    tx_index: index,
                    error: result.execution_info.revert_error.as_ref().map(|e| e.to_string()).unwrap_or_default(),
                });
            }
            acc.push(exec_context.execution_result_to_fee_estimate(result));
            Ok(acc)
        },
    )?;

    Ok(fee_estimates)
}
