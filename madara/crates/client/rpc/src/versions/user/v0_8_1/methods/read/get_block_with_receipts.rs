use mp_block::MadaraMaybePendingBlockInfo;
use mp_rpc::v0_8_1::{
    BlockId, BlockStatus, BlockWithReceipts, PendingBlockWithReceipts, StarknetGetBlockWithTxsAndReceiptsResult,
    TransactionAndReceipt, TxnFinalityStatus,
};

use crate::errors::StarknetRpcResult;
use crate::Starknet;

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
    tracing::debug!("get_block_with_receipts called with {:?}", block_id);
    let block = starknet.get_block(&block_id)?;

    let transactions = block.inner.transactions.into_iter().map(|tx| tx.into());

    let is_on_l1 = if let Some(block_n) = block.info.block_n() {
        block_n <= starknet.get_l1_last_confirmed_block()?
    } else {
        false
    };

    let finality_status = if is_on_l1 { TxnFinalityStatus::L1 } else { TxnFinalityStatus::L2 };

    let receipts = block.inner.receipts.into_iter().map(|receipt| receipt.to_starknet_types(finality_status.clone()));

    let transactions_with_receipts = Iterator::zip(transactions, receipts)
        .map(|(transaction, receipt)| TransactionAndReceipt { receipt, transaction })
        .collect();

    match block.info {
        MadaraMaybePendingBlockInfo::Pending(block_info) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Pending(PendingBlockWithReceipts {
                transactions: transactions_with_receipts,
                pending_block_header: block_info.into(),
            }))
        }
        MadaraMaybePendingBlockInfo::NotPending(block_info) => {
            let status = if is_on_l1 { BlockStatus::AcceptedOnL1 } else { BlockStatus::AcceptedOnL2 };
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                transactions: transactions_with_receipts,
                status,
                block_header: block_info.into(),
            }))
        }
    }
}
