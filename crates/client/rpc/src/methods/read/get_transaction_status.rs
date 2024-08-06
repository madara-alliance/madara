use dp_block::DeoxysMaybePendingBlockInfo;
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
    let (block, tx_index) = starknet
        .backend
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error find tx hash block info from db")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    // Note: we don't support TransactionStatus::Received and TransactionStatus::Rejected yet.

    let tx_receipt = block.inner.receipts.get(tx_index.0 as usize).ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_execution_status = match tx_receipt.execution_result() {
        ExecutionResult::Reverted { .. } => TransactionExecutionStatus::Reverted,
        ExecutionResult::Succeeded => TransactionExecutionStatus::Succeeded,
    };

    match block.info {
        DeoxysMaybePendingBlockInfo::Pending(_) => Ok(TransactionStatus::AcceptedOnL2(tx_execution_status)),
        DeoxysMaybePendingBlockInfo::NotPending(block) => {
            if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                Ok(TransactionStatus::AcceptedOnL1(tx_execution_status))
            } else {
                Ok(TransactionStatus::AcceptedOnL2(tx_execution_status))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{make_sample_chain_1, open_testing, SampleChain1};
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_status() {
        let _ = env_logger::builder().is_test(true).try_init();
        let (backend, rpc) = open_testing();
        let SampleChain1 { tx_hashes, .. } = make_sample_chain_1(&backend);

        // Block 0
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[0]).unwrap(),
            TransactionStatus::AcceptedOnL1(TransactionExecutionStatus::Succeeded)
        );

        // Block 1

        // Block 2
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[1]).unwrap(),
            TransactionStatus::AcceptedOnL2(TransactionExecutionStatus::Succeeded)
        );
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[2]).unwrap(),
            TransactionStatus::AcceptedOnL2(TransactionExecutionStatus::Reverted)
        );

        // Pending
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[3]).unwrap(),
            TransactionStatus::AcceptedOnL2(TransactionExecutionStatus::Succeeded)
        );
    }
}
