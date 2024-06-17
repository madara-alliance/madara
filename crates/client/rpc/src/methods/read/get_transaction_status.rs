use dp_convert::felt_wrapper::FeltWrapper;
use jsonrpsee::core::RpcResult;
use starknet_api::transaction::TransactionHash;
use starknet_core::types::{Felt, TransactionExecutionStatus, TransactionStatus};

use crate::errors::StarknetRpcApiError;
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
pub fn get_transaction_status(starknet: &Starknet, transaction_hash: Felt) -> RpcResult<TransactionStatus> {
    let tx_hash = TransactionHash(transaction_hash.into_stark_felt());

    let block = starknet
        .block_storage()
        .find_tx_hash_block_info(&tx_hash)
        .or_internal_server_error("Error find tx hash block info from db")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_execution_status = match starknet
        .block_storage()
        .is_reverted_tx(&tx_hash)
        .or_internal_server_error("Error getting tx status from db")?
    {
        true => TransactionExecutionStatus::Reverted,
        false => TransactionExecutionStatus::Succeeded,
    };

    if block.0.block_n() > starknet.get_l1_last_confirmed_block()? {
        Ok(TransactionStatus::AcceptedOnL2(tx_execution_status))
    } else {
        Ok(TransactionStatus::AcceptedOnL1(tx_execution_status))
    }
}
