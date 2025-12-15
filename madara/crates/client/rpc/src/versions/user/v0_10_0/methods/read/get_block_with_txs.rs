use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithTxs, MaybePreConfirmedBlockWithTxs, PreConfirmedBlockWithTxs, TxnWithHash,
};

pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<MaybePreConfirmedBlockWithTxs> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let transactions_with_hash = view
        .get_executed_transactions(..)?
        .into_iter()
        .map(|tx| TxnWithHash {
            transaction: tx.transaction.to_rpc_v0_8(),
            transaction_hash: *tx.receipt.transaction_hash(),
        })
        .collect();

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxs::PreConfirmed(PreConfirmedBlockWithTxs {
                transactions: transactions_with_hash,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxs::Block(BlockWithTxs {
                transactions: transactions_with_hash,
                status,
                block_header: block.to_rpc_v0_10(),
            }))
        }
    }
}
