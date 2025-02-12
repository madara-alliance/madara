use mc_exec::transaction::to_blockifier_transaction;
use mc_exec::{execution_result_to_tx_trace, ExecutionContext};
use mp_block::BlockId;
use mp_convert::ToFelt;
use mp_rpc::TraceBlockTransactionsResult;
use starknet_api::transaction::TransactionHash;
use std::sync::Arc;

use super::trace_transaction::EXECUTION_UNSUPPORTED_BELOW_VERSION;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<TraceBlockTransactionsResult>> {
    let block = starknet.get_block(&block_id)?;

    if block.info.protocol_version() < &EXECUTION_UNSUPPORTED_BELOW_VERSION {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let exec_context = ExecutionContext::new_at_block_start(Arc::clone(&starknet.backend), &block.info)?;

    let transactions: Vec<_> = block
        .inner
        .transactions
        .into_iter()
        .zip(block.info.tx_hashes())
        .map(|(tx, hash)| {
            to_blockifier_transaction(starknet.clone_backend(), block_id.clone(), tx, &TransactionHash(*hash))
                .or_internal_server_error("Failed to convert transaction to blockifier format")
        })
        .collect::<Result<_, _>>()?;

    let executions_results = exec_context.re_execute_transactions([], transactions, true, true)?;

    let traces = executions_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            let trace_root = execution_result_to_tx_trace(&result)
                .or_internal_server_error("Converting execution infos to tx trace")?;
            Ok(TraceBlockTransactionsResult { trace_root, transaction_hash })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(traces)
}
