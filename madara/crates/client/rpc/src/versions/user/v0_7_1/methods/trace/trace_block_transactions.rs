use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::Starknet;
use anyhow::Context;
use mc_db::MadaraStateView;
use mc_exec::{execution_result_to_tx_trace, MadaraBlockViewExecutionExt, EXECUTION_UNSUPPORTED_BELOW_VERSION};
use mp_block::{BlockId, TransactionWithReceipt};
use mp_convert::ToFelt;
use mp_rpc::TraceBlockTransactionsResult;
use mp_transactions::TransactionWithHash;

pub(super) fn prepare_tx_for_reexecution(
    view: &MadaraStateView,
    tx: TransactionWithReceipt,
) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
    let class = if let Some(tx) = tx.transaction.as_declare() {
        Some(
            view.get_class_info_and_compiled(tx.class_hash())?
                .with_context(|| format!("No class found for class_hash={:#x}", tx.class_hash()))?,
        )
    } else {
        None
    };

    TransactionWithHash::new(tx.transaction, *tx.receipt.transaction_hash())
        .into_blockifier(class.as_ref())
        .context("Error converting transaction to blockifier format for reexecution")
}

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<TraceBlockTransactionsResult>> {
    let view = starknet.backend.block_view(block_id)?;
    let mut exec_context = view.new_execution_context_at_block_start()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    let state_view = view.state_view();
    let transactions: Vec<_> = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| prepare_tx_for_reexecution(&state_view, tx))
        .collect::<Result<_, _>>()?;

    let executions_results =
        mp_utils::spawn_blocking(move || exec_context.execute_transactions([], transactions)).await?;

    let traces = executions_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            let trace_root = execution_result_to_tx_trace(&result).context("Converting execution infos to tx trace")?;
            Ok(TraceBlockTransactionsResult { trace_root, transaction_hash })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(traces)
}
