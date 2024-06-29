use dc_exec::{block_context, execute_transactions, execution_result_to_tx_trace};
use dp_convert::{ToFelt, ToStarkFelt};
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::transaction::to_blockifier_transactions;
use crate::utils::ResultExt;
use crate::Starknet;

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<Vec<TransactionTraceWithHash>> {
    let block = starknet.get_block(block_id)?;

    if block.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion);
    }

    let block_context = block_context(block.header(), &starknet.chain_id());

    let transactions: Vec<_> = block
        .transactions()
        .iter()
        .zip(block.tx_hashes())
        .map(|(tx, hash)| to_blockifier_transactions(starknet, tx, &TransactionHash(hash.to_stark_felt())))
        .collect::<Result<_, _>>()?;

    let executions_results =
        execute_transactions(starknet.clone_backend(), [], transactions, &block_context, true, true)
            .or_internal_server_error("Failed to re-execute transactions")?;

    let traces = executions_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            let trace_root = execution_result_to_tx_trace(&result).map_err(|e| {
                StarknetRpcApiError::ErrUnexpectedError { data: format!("Converting execution infos to tx trace: {e}") }
            })?;
            Ok(TransactionTraceWithHash { trace_root, transaction_hash })
        })
        .collect::<Result<Vec<_>, StarknetRpcApiError>>()?;

    Ok(traces)
}
