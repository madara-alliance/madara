use mc_db::MadaraStorageRead;
use mc_exec::execution::TxInfo;
use mc_exec::{ExecutionContext, ExecutionResult};
use mp_transactions::L1HandlerTransaction;
use starknet_api::transaction::fields::{Fee, Tip};
use starknet_api::transaction::TransactionHash;
use starknet_types_core::felt::Felt;

/// Shared execution path of `starknet_estimateMessageFee`, used by every RPC version: builds the
/// executable L1 handler transaction and executes it against the given context. Revert surfacing
/// and fee conversion differ by RPC version and stay in the versioned handlers.
pub async fn execute_message_fee_estimation<D: MadaraStorageRead>(
    mut exec_context: ExecutionContext<D>,
    l1_handler: L1HandlerTransaction,
    chain_id: Felt,
) -> Result<(ExecutionResult, ExecutionContext<D>, Tip), mc_exec::Error> {
    let tx_hash = l1_handler.compute_hash(chain_id, /* offset_version */ false, /* legacy */ false);
    let tx: starknet_api::transaction::L1HandlerTransaction = l1_handler
        .try_into()
        .map_err(|err| mc_exec::Error::Internal(anyhow::anyhow!("Converting L1 handler transaction: {err:#}")))?;
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

    let execution_result = execution_results
        .pop()
        .ok_or_else(|| mc_exec::Error::Internal(anyhow::anyhow!("There should be one result")))?;

    Ok((execution_result, exec_context, tip))
}
