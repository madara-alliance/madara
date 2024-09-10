use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::transaction::to_blockifier_transactions;
use crate::utils::ResultExt;
use crate::Starknet;
use mc_exec::{execution_result_to_tx_trace, ExecutionContext};
use mp_convert::ToFelt;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{BlockId, TransactionTraceWithHash};
use std::sync::Arc;

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<TransactionTraceWithHash>> {
    let block = starknet.get_block(&block_id)?;

    if block.info.protocol_version() < &FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let exec_context = ExecutionContext::new_in_block(Arc::clone(&starknet.backend), &block.info)?;

    let transactions: Vec<_> = block
        .inner
        .transactions
        .into_iter()
        .zip(block.info.tx_hashes())
        .map(|(tx, hash)| to_blockifier_transactions(starknet, block_id.into(), tx, &TransactionHash(*hash)))
        .collect::<Result<_, _>>()?;

    let executions_results = exec_context.re_execute_transactions([], transactions, true, true)?;

    let traces = executions_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            let trace_root = execution_result_to_tx_trace(&result)
                .or_internal_server_error("Converting execution infos to tx trace")?;
            Ok(TransactionTraceWithHash { trace_root, transaction_hash })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(traces)
}
