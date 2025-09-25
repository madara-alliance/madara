use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::tx_api_to_blockifier;
use crate::Starknet;
use anyhow::Context;
use blockifier::transaction::account_transaction::ExecutionFlags;
use mc_exec::execution::TxInfo;
use mc_exec::trace::execution_result_to_tx_trace_v0_8;
use mc_exec::{MadaraBlockViewExecutionExt, EXECUTION_UNSUPPORTED_BELOW_VERSION};
use mp_convert::ToFelt;
use mp_rpc::v0_8_1::{BlockId, BroadcastedTxn, SimulateTransactionsResult, SimulationFlag};
use mp_transactions::{IntoStarknetApiExt, ToBlockifierError};

pub async fn simulate_transactions(
    starknet: &Starknet,
    block_id: BlockId,
    transactions: Vec<BroadcastedTxn>,
    simulation_flags: Vec<SimulationFlag>,
) -> StarknetRpcResult<Vec<SimulateTransactionsResult>> {
    let view = starknet.resolve_block_view(block_id)?;
    let mut exec_context = view.new_execution_context()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let charge_fee = !simulation_flags.contains(&SimulationFlag::SkipFeeCharge);
    let validate = !simulation_flags.contains(&SimulationFlag::SkipValidate);

    let user_transactions = transactions
        .into_iter()
        .map(|tx| {
            let only_query = tx.is_query();
            let (api_tx, _) = tx
                .into_starknet_api(starknet.backend.chain_config().chain_id.to_felt(), exec_context.protocol_version)?;
            let execution_flags = ExecutionFlags { only_query, charge_fee, validate, strict_nonce_check: true };
            Ok(tx_api_to_blockifier(api_tx, execution_flags)?)
        })
        .collect::<Result<Vec<_>, ToBlockifierError>>()?;

    let tips = user_transactions.iter().map(|tx| tx.tip().unwrap_or_default()).collect::<Vec<_>>();

    // spawn_blocking: avoid starving the tokio workers during execution.
    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], user_transactions)?, exec_context))
    })
    .await?;

    let simulated_transactions = execution_results
        .iter()
        .zip(tips)
        .map(|(result, tip)| {
            Ok(SimulateTransactionsResult {
                transaction_trace: execution_result_to_tx_trace_v0_8(
                    result,
                    exec_context.block_context.versioned_constants(),
                )
                .context("Converting execution infos to tx trace")?,
                fee_estimation: exec_context
                    .execution_result_to_fee_estimate_v0_8(result, tip)
                    .context("Converting execution infos to tx trace")?,
            })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(simulated_transactions)
}
