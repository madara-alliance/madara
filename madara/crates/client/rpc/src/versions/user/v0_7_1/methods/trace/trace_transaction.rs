use crate::errors::StarknetRpcResult;
use crate::versions::user::v0_7_1::methods::trace::trace_block_transactions::prepare_tx_for_reexecution;
use crate::{Starknet, StarknetRpcApiError};
use anyhow::Context;
use mc_db::MadaraBlockView;
use mc_exec::{execution_result_to_tx_trace, MadaraBlockViewExecutionExt, EXECUTION_UNSUPPORTED_BELOW_VERSION};
use mp_rpc::TraceTransactionResult;
use starknet_types_core::felt::Felt;

pub async fn trace_transaction(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TraceTransactionResult> {
    let view = starknet.backend.view_on_latest();
    let tx_index = view.find_transaction_by_hash(&transaction_hash)?.ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let block_view: MadaraBlockView = if !tx_index.is_preconfirmed {
        view.block_view_on_confirmed(tx_index.block_number).context("Block should be found")?.into()
    } else {
        // reconfirmed must be latest
        view.block_view_on_latest().context("View can't be empty after finding a transaction")?.clone()
    };
    let mut exec_context = block_view.new_execution_context_at_block_start()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = block_view.state_view();
    // Takes up until but not including the transaction we're interested in.
    let previous_transactions: Vec<_> = block_view
        .get_executed_transactions(..tx_index.block_number)?
        .into_iter()
        .map(|tx| prepare_tx_for_reexecution(&state_view, tx))
        .collect::<Result<_, _>>()?;

    let transaction_to_trace = prepare_tx_for_reexecution(
        &state_view,
        block_view.get_executed_transaction(tx_index.block_number)?.context("Transaction should exist")?,
    )?;

    // Reexecute all transactions before the one we're interested in, and trace the one we're interested in.
    let mut executions_results = mp_utils::spawn_blocking(move || {
        exec_context.execute_transactions(previous_transactions, [transaction_to_trace])
    })
    .await?;

    let execution_result = executions_results.pop().context("No execution info returned")?;

    let trace = execution_result_to_tx_trace(&execution_result).context("Converting execution infos to tx trace")?;

    Ok(TraceTransactionResult { trace })
}
