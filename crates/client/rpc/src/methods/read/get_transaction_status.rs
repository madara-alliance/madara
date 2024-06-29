use dp_receipt::ExecutionResult;
use starknet_core::types::{Felt, TransactionExecutionStatus, TransactionStatus};

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Gets the Transaction Status, Including Mempool Status and Execution Details
///
/// This method retrieves the status of a specified transaction. It provides information on
/// whether the transaction is still in the mempool, has been executed, or dropped from the
/// mempool. The status includes both finality status and execution status of the
/// transaction.
///
/// ### Arguments
///
/// * `transaction_hash` - The hash of the transaction for which the status is requested.
///
/// ### Returns
///
/// * `transaction_status` - An object containing the transaction status details:
///   - `finality_status`: The finality status of the transaction, indicating whether it is
///     confirmed, pending, or rejected.
///   - `execution_status`: The execution status of the transaction, providing details on the
///     execution outcome if the transaction has been processed.
pub fn get_transaction_status(starknet: &Starknet, transaction_hash: Felt) -> StarknetRpcResult<TransactionStatus> {
    let (block, tx_torage_info) = starknet
        .block_storage()
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error find tx hash block info from db")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_receipt = block.receipts().get(tx_torage_info.tx_index).ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_execution_status = match tx_receipt.execution_result() {
        ExecutionResult::Reverted { .. } => TransactionExecutionStatus::Reverted,
        ExecutionResult::Succeeded => TransactionExecutionStatus::Succeeded,
    };

    if block.block_n() <= starknet.get_l1_last_confirmed_block()? {
        Ok(TransactionStatus::AcceptedOnL1(tx_execution_status))
    } else {
        Ok(TransactionStatus::AcceptedOnL2(tx_execution_status))
    }
}
