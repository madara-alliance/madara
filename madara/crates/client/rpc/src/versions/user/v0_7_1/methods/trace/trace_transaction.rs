use crate::errors::StarknetRpcResult;
use crate::versions::user::v0_7_1::methods::trace::trace_block_transactions::prepare_tx_for_reexecution;
use crate::{Starknet, StarknetRpcApiError};
use anyhow::Context;
use mc_exec::{execution_result_to_tx_trace, MadaraBlockViewExecutionExt, EXECUTION_UNSUPPORTED_BELOW_VERSION};
use mp_rpc::v0_7_1::TraceTransactionResult;
use starknet_types_core::felt::Felt;

pub async fn trace_transaction(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TraceTransactionResult> {
    let view = starknet.backend.view_on_latest();
    let res = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;
    let mut exec_context = res.block.new_execution_context_at_block_start()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = res.block.state_view();
    // Takes up until but not including the transaction we're interested in.
    let previous_transactions: Vec<_> = res
        .block
        .get_executed_transactions(..res.transaction_index)?
        .into_iter()
        .map(|tx| prepare_tx_for_reexecution(&state_view, tx))
        .collect::<Result<_, _>>()?;

    let transaction_to_trace = prepare_tx_for_reexecution(&state_view, res.get_transaction()?)?;

    // Reexecute all transactions before the one we're interested in, and trace the one we're interested in.
    let mut executions_results = mp_utils::spawn_blocking(move || {
        exec_context.execute_transactions(previous_transactions, [transaction_to_trace])
    })
    .await?;

    let execution_result = executions_results.pop().context("No execution info returned")?;

    let trace = execution_result_to_tx_trace(&execution_result).context("Converting execution infos to tx trace")?;

    Ok(TraceTransactionResult { trace })
}
