use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithReceipts, PreConfirmedBlockWithReceipts, StarknetGetBlockWithTxsAndReceiptsResult,
    TransactionAndReceipt,
};

pub fn get_block_with_receipts(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<StarknetGetBlockWithTxsAndReceiptsResult> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    let transactions_with_receipts = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TransactionAndReceipt {
            receipt: tx.receipt.to_rpc_v0_9(status.into()),
            transaction: tx.transaction.to_rpc_v0_8(),
        })
        .collect();

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::PreConfirmed(PreConfirmedBlockWithReceipts {
                transactions: transactions_with_receipts,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            Ok(StarknetGetBlockWithTxsAndReceiptsResult::Block(BlockWithReceipts {
                transactions: transactions_with_receipts,
                status,
                block_header: block.to_rpc_v0_10(),
            }))
        }
    }
}
