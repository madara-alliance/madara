use mp_block::MadaraMaybePendingBlockInfo;
use mp_receipt::ExecutionResult;
use starknet_types_core::felt::Felt;
use starknet_types_rpc::{TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnStatus};

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
pub fn get_transaction_status(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TxnFinalityAndExecutionStatus> {
    let (block, tx_index) = starknet
        .backend
        .find_tx_hash_block(&transaction_hash)
        .or_internal_server_error("Error find tx hash block info from db")?
        .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    // Note: we don't support TransactionStatus::Received and TransactionStatus::Rejected yet.

    let tx_receipt = block.inner.receipts.get(tx_index.0 as usize).ok_or(StarknetRpcApiError::TxnHashNotFound)?;

    let tx_execution_status = match tx_receipt.execution_result() {
        ExecutionResult::Reverted { .. } => TxnExecutionStatus::Reverted,
        ExecutionResult::Succeeded => TxnExecutionStatus::Succeeded,
    };

    let finality_status = match block.info {
        MadaraMaybePendingBlockInfo::Pending(_) => TxnStatus::AcceptedOnL2,
        MadaraMaybePendingBlockInfo::NotPending(block) => {
            if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                TxnStatus::AcceptedOnL1
            } else {
                TxnStatus::AcceptedOnL2
            }
        }
    };

    Ok(TxnFinalityAndExecutionStatus { finality_status, execution_status: Some(tx_execution_status) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};
    use rstest::rstest;

    #[rstest]
    fn test_get_transaction_status(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[0]).unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL1,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );

        // Block 1

        // Block 2
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[1]).unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[2]).unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(TxnExecutionStatus::Reverted)
            }
        );

        // Pending
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[3]).unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );
    }

    #[rstest]
    fn test_get_transaction_status_not_found(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_status(&rpc, does_not_exist), Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
