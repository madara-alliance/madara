use blockifier::transaction::account_transaction::AccountTransaction;
use jsonrpsee::core::RpcResult;
use mp_felt::FeltWrapper;
use mp_transactions::TxType;
use starknet_api::transaction::{Transaction, TransactionHash};
use starknet_core::types::TransactionTraceWithHash;
use starknet_ff::FieldElement;

use super::super::read::get_transaction_receipt::execution_infos;
use super::utils::tx_execution_infos_to_tx_trace;
use crate::errors::StarknetRpcApiError;
use crate::utils::execution::block_context;
use crate::utils::transaction::blockifier_transactions;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

pub async fn trace_transaction(
    starknet: &Starknet,
    transaction_hash: FieldElement,
) -> RpcResult<TransactionTraceWithHash> {
    let transaction_hash_f = TransactionHash(transaction_hash.into_stark_felt());
    let block = starknet
        .block_storage()
        .get_block_from_tx_hash(&transaction_hash_f)
        .or_internal_server_error("Error while getting block from tx hash")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let block_hash = block.block_hash().into_field_element();
    let block_number = block.block_n();
    let block_context = block_context(starknet, block.info())?;

    // retrieve the transaction index in the block with the transaction hash
    let (tx_index, _) = block
        .tx_hashes()
        .iter()
        .enumerate()
        .find(|(_, hash)| *hash == &transaction_hash_f)
        .ok_or_else_internal_server_error(|| {
            format!("Failed to retrieve transaction index from block with hash {block_hash:#x}")
        })?;

    // create a vector of tuples with the transaction and its hash, up to the current transaction index
    let transaction_with_hash = block
        .transactions()
        .iter()
        .cloned()
        .zip(block.tx_hashes())
        .filter(|(tx, _)| !matches!(tx, Transaction::Deploy(_)))
        .take(tx_index + 1)
        .map(|(tx, tx_hash)| (tx, tx_hash.into_field_element()))
        .collect();

    let transactions_blockifier = blockifier_transactions(transaction_with_hash)?;

    let last_transaction =
        transactions_blockifier.last().ok_or_internal_server_error("There should be at least one transaction")?;

    let tx_type = match last_transaction {
        blockifier::transaction::transaction_execution::Transaction::AccountTransaction(account_tx) => match account_tx
        {
            AccountTransaction::Declare(_) => TxType::Declare,
            AccountTransaction::DeployAccount(_) => TxType::DeployAccount,
            AccountTransaction::Invoke(_) => TxType::Invoke,
        },
        blockifier::transaction::transaction_execution::Transaction::L1HandlerTransaction(_) => TxType::L1Handler,
    };

    let execution_infos = execution_infos(transactions_blockifier, &block_context)?;

    let trace = tx_execution_infos_to_tx_trace(tx_type, &execution_infos, block_number).unwrap();

    let tx_trace = TransactionTraceWithHash { transaction_hash, trace_root: trace };

    Ok(tx_trace)
}
