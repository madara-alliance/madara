use dp_convert::core_felt::CoreFelt;
use dp_transactions::TxType;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::Transaction;
use starknet_core::types::{BlockId, TransactionTraceWithHash};

use super::utils::tx_execution_infos_to_tx_trace;
use crate::utils::execution::{block_context, re_execute_transactions};
use crate::utils::transaction::blockifier_transactions;
use crate::utils::ResultExt;
use crate::Starknet;

pub async fn trace_block_transactions(
    starknet: &Starknet,
    block_id: BlockId,
) -> RpcResult<Vec<TransactionTraceWithHash>> {
    let block = starknet.get_block(block_id)?;

    let block_number = block.block_n();
    let block_context = block_context(starknet, block.info())?;

    let block_txs_hashes = block.tx_hashes().iter().map(CoreFelt::into_core_felt);

    // create a vector of transactions with their corresponding hashes without deploy transactions,
    // blockifier does not support deploy transactions
    let transaction_with_hash: Vec<_> = block
        .transactions()
        .iter()
        .cloned()
        .zip(block_txs_hashes)
        .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_)))
        .collect();

    let transactions_blockifier = blockifier_transactions(starknet, transaction_with_hash.clone())?;

    let mut transactions_traces = Vec::new();

    let execution_infos = re_execute_transactions(starknet, vec![], transactions_blockifier, &block_context)
        .or_internal_server_error("Failed to re-execute transactions")?;

    for ((transaction, tx_hash), tx_exec_info) in transaction_with_hash.iter().zip(execution_infos) {
        let tx_type = match transaction {
            Transaction::Declare(_) => TxType::Declare,
            Transaction::DeployAccount(_) => TxType::DeployAccount,
            Transaction::Invoke(_) => TxType::Invoke,
            Transaction::L1Handler(_) => TxType::L1Handler,
            Transaction::Deploy(_) => unreachable!(),
        };

        let trace = tx_execution_infos_to_tx_trace(starknet, tx_type, &tx_exec_info, block_number)
            .or_internal_server_error("Failed to generate trace")?;
        let transaction_trace = TransactionTraceWithHash { trace_root: trace, transaction_hash: *tx_hash };
        transactions_traces.push(transaction_trace);
    }

    Ok(transactions_traces)
}
