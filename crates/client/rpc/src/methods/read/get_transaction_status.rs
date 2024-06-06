use jsonrpsee::core::RpcResult;
use starknet_core::types::{FieldElement, TransactionStatus};

use crate::errors::StarknetRpcApiError;
use crate::utils::helpers::{block_hash_from_block_n, txs_hashes_from_block_hash};
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
pub fn get_transaction_status(_starknet: &Starknet, _transaction_hash: FieldElement) -> RpcResult<TransactionStatus> {
    // let transaction_hash = TransactionHash(transaction_hash.into_stark_felt());

    // let block = starknet
    //     .block_storage()
    //     .get_block_info_from_tx_hash(&transaction_hash)?
    //     .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    // let _starknet_tx = starknet
    //     .get_block_transaction_hashes(starknet_block_hash.into())?
    //     .into_iter()
    //     .zip(starknet_block.transactions())
    //     .find(|(tx_hash, _)| *tx_hash == Felt252Wrapper(transaction_hash).into())
    //     .map(|(_, tx)| to_starknet_core_tx(tx.clone(), transaction_hash));

    // TODO: Implement this method
    Err(StarknetRpcApiError::UnimplementedMethod.into())

    // let execution_status = {
    //     let revert_error = starknet
    //         .client
    //         .runtime_api()
    //         .get_tx_execution_outcome(substrate_block_hash,
    // Felt252Wrapper(transaction_hash).into())         .map_err(|e| {
    //             log::error!(
    //                 "Failed to get transaction execution outcome. Substrate block hash:
    // {substrate_block_hash}, \                  transaction hash: {transaction_hash}, error:
    // {e}"             );
    //             StarknetRpcApiError::InternalServerError
    //         })?;

    //     if revert_error.is_none() {
    //         TransactionExecutionStatus::Succeeded
    //     } else {
    //         TransactionExecutionStatus::Reverted
    //     }
    // };

    // Ok(TransactionStatus::AcceptedOnL2(execution_status))
}
