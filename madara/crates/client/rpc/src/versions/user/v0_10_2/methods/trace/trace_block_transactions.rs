use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::display_internal_server_error;
use crate::Starknet;
use anyhow::Context;
use mc_db::MadaraStateView;
use mc_exec::trace::execution_result_to_tx_trace_v0_9;
use mc_exec::{state_maps_to_initial_reads, MadaraBlockViewExecutionExt, EXECUTION_UNSUPPORTED_BELOW_VERSION};
use mp_block::TransactionWithReceipt;
use mp_convert::ToFelt;
use mp_rpc::v0_10_0::BlockId;
use mp_rpc::v0_10_2::{TraceBlockTransactionsResponse, TraceBlockTransactionsResult, TraceFlag};
use mp_transactions::TransactionWithHash;

fn prepare_tx_for_reexecution(
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
    trace_flags: Option<Vec<TraceFlag>>,
) -> StarknetRpcResult<TraceBlockTransactionsResponse> {
    let view = starknet.resolve_block_view(block_id)?;
    let mut exec_context = view.new_execution_context_at_block_start()?;

    if exec_context.protocol_version < EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::unsupported_txn_version());
    }

    // Check if RETURN_INITIAL_READS flag is set
    let return_initial_reads = trace_flags
        .as_ref()
        .map(|flags| flags.iter().any(|f| matches!(f, TraceFlag::ReturnInitialReads)))
        .unwrap_or(false);

    let state_view = view.state_view();
    let transactions: Vec<_> = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| prepare_tx_for_reexecution(&state_view, tx))
        .collect::<Result<_, _>>()?;

    let (execution_results, exec_context) = mp_utils::spawn_blocking(move || {
        Ok::<_, mc_exec::Error>((exec_context.execute_transactions([], transactions)?, exec_context))
    })
    .await?;

    // Get initial reads if requested
    let initial_reads = if return_initial_reads {
        match exec_context.get_initial_reads() {
            Ok(state_maps) => Some(state_maps_to_initial_reads(state_maps)),
            Err(e) => {
                display_internal_server_error(format!("Failed to get initial reads: {e}"));
                return Err(StarknetRpcApiError::InternalServerError);
            }
        }
    } else {
        None
    };

    let traces = execution_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            let trace_root =
                execution_result_to_tx_trace_v0_9(&result, exec_context.block_context.versioned_constants())
                    .context("Converting execution infos to tx trace")?
                    .into();
            Ok(TraceBlockTransactionsResult { trace_root, transaction_hash })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(TraceBlockTransactionsResponse::new(traces, initial_reads))
}
