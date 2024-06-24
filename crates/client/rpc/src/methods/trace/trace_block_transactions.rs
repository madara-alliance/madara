use blockifier::transaction::account_transaction::AccountTransaction;
use dp_convert::ToStarkFelt;
use dp_transactions::TxType;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::trace_transaction::FALLBACK_TO_SEQUENCER_WHEN_VERSION_BELOW;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::{block_context, re_execute_transactions};
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

    use blockifier::transaction::transaction_execution::Transaction as BTx;
    let types: Vec<_> = transactions
        .iter()
        .map(|tx| match tx {
            BTx::AccountTransaction(account_tx) => match account_tx {
                AccountTransaction::Declare(_) => TxType::Declare,
                AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
                AccountTransaction::Invoke(_) => TxType::Invoke,
            },
            BTx::L1HandlerTransaction(_) => TxType::L1Handler,
        })
        .collect();

    let execution_infos = re_execute_transactions(starknet, [], transactions, &block_context)
        .or_internal_server_error("Failed to re-execute transactions")?;

    let traces = execution_infos
        .iter()
        .zip(types)
        .zip(block.tx_hashes())
        .map(|((info, ty), tx_hash)| {
            tx_execution_infos_to_tx_trace(starknet, ty, info, block.block_n())
                .or_internal_server_error("Converting execution infos to tx trace")
                .map(|trace| TransactionTraceWithHash { trace_root: trace, transaction_hash: *tx_hash })
        })
        .collect::<Result<_, _>>()?;

    Ok(traces)
}
