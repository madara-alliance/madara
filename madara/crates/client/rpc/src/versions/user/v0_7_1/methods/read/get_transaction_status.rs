use mp_block::MadaraMaybePendingBlockInfo;
use mp_receipt::ExecutionResult;
use mp_rpc::{TxnExecutionStatus, TxnFinalityAndExecutionStatus, TxnStatus};
use starknet_types_core::felt::Felt;

use crate::errors::{StarknetRpcApiError, StarknetRpcResult};
use crate::utils::ResultExt;
use crate::Starknet;

/// Gets the status of a transaction. ([specs])
///
/// Supported statuses are:
///
/// - [`Received`]: tx has been inserted into the mempool.
/// - [`AcceptedOnL2`]: tx has been saved to a closed block.
/// - [`AcceptedOnL1`]: tx has been finalized on L1.
///
/// We do not count a transaction as being accepted on L2 if it is included in the pending block.
/// This is because the pending block is not a reliable storage of information: it is not persisted
/// between restarts and as such can be lost in case of a node shutdown, and in the future it could
/// be dropped in case of consensus disagreements. A transaction being included in the pending block
/// therefore is _not_ a guarantee that it will necessarily be accepted on L1.
///
/// We do not currently support the **Rejected** transaction status.
///
/// [specs]: https://github.com/starkware-libs/starknet-specs/blob/a2d10fc6cbaddbe2d3cf6ace5174dd0a306f4885/api/starknet_api_openrpc.json#L224C5-L250C7
/// [`Received`]: mp_rpc::v0_7_1::TxnStatus::Received
/// [`AcceptedOnL2`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL2
/// [`AcceptedOnL1`]: mp_rpc::v0_7_1::TxnStatus::AcceptedOnL1
pub async fn get_transaction_status(
    starknet: &Starknet,
    transaction_hash: Felt,
) -> StarknetRpcResult<TxnFinalityAndExecutionStatus> {
    let (finality_status, execution_status) =
        if starknet.add_transaction_provider.received_transaction(transaction_hash).await.is_some_and(|b| b) {
            (TxnStatus::Received, None)
        } else {
            let (block, tx_index) = starknet
                .backend
                .find_tx_hash_block(&transaction_hash)
                .or_else_internal_server_error(|| {
                    format!("GetTransactionStatus failed to retrieve block for tx {transaction_hash:#x}")
                })?
                .ok_or(StarknetRpcApiError::TxnHashNotFound)?;

            let tx_receipt =
                block.inner.receipts.get(tx_index.0 as usize).ok_or(StarknetRpcApiError::TxnHashNotFound)?;

            let execution_status = match tx_receipt.execution_result() {
                ExecutionResult::Reverted { .. } => Some(TxnExecutionStatus::Reverted),
                ExecutionResult::Succeeded => Some(TxnExecutionStatus::Succeeded),
            };

            let finality_status = match block.info {
                MadaraMaybePendingBlockInfo::Pending(_) => TxnStatus::Received,
                MadaraMaybePendingBlockInfo::NotPending(block) => {
                    if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                        TxnStatus::AcceptedOnL1
                    } else {
                        TxnStatus::AcceptedOnL2
                    }
                }
            };
            (finality_status, execution_status)
        };

    Ok(TxnFinalityAndExecutionStatus { finality_status, execution_status })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{sample_chain_for_block_getters, SampleChainForBlockGetters};

    #[tokio::test]
    #[rstest::rstest]
    async fn test_get_transaction_status(sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet)) {
        let (SampleChainForBlockGetters { tx_hashes, .. }, rpc) = sample_chain_for_block_getters;

        // Block 0
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[0]).await.unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL1,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );

        // Block 1

        // Block 2
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[1]).await.unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[2]).await.unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::AcceptedOnL2,
                execution_status: Some(TxnExecutionStatus::Reverted)
            }
        );

        // Pending
        assert_eq!(
            get_transaction_status(&rpc, tx_hashes[3]).await.unwrap(),
            TxnFinalityAndExecutionStatus {
                finality_status: TxnStatus::Received,
                execution_status: Some(TxnExecutionStatus::Succeeded)
            }
        );
    }

    #[tokio::test]
    #[rstest::rstest]
    async fn test_get_transaction_status_not_found(
        sample_chain_for_block_getters: (SampleChainForBlockGetters, Starknet),
    ) {
        let (SampleChainForBlockGetters { .. }, rpc) = sample_chain_for_block_getters;

        let does_not_exist = Felt::from_hex_unchecked("0x7128638126378");
        assert_eq!(get_transaction_status(&rpc, does_not_exist).await, Err(StarknetRpcApiError::TxnHashNotFound));
    }
}
