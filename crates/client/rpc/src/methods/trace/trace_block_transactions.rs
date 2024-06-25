use dp_convert::{ToFelt, ToStarkFelt};
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::{block_context, execute_transactions};
use crate::utils::transaction::to_blockifier_transactions;
use crate::utils::ResultExt;
use crate::Starknet;

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>> {
    let block = starknet.get_block(block_id)?;

    if block.header().protocol_version < FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW {
        return Err(StarknetRpcApiError::UnsupportedTxnVersion.into());
    }

    let block_context = block_context(starknet, block.info())?;

    let transactions: Vec<_> = block
        .transactions()
        .iter()
        .zip(block.tx_hashes())
        .map(|(tx, hash)| to_blockifier_transactions(starknet, tx, &TransactionHash(hash.to_stark_felt())))
        .collect::<Result<_, _>>()?;

    let executions_results = execute_transactions(starknet, [], transactions, &block_context)
        .or_internal_server_error("Failed to re-execute transactions")?;

    let traces = executions_results
        .into_iter()
        .map(|result| {
            let transaction_hash = result.hash.to_felt();
            tx_execution_infos_to_tx_trace(starknet, &result, block.block_n())
                .or_internal_server_error("Converting execution infos to tx trace")
                .map(|trace_root| TransactionTraceWithHash { trace_root, transaction_hash })
        })
        .collect::<Result<_, _>>()?;

    Ok(traces)
}
