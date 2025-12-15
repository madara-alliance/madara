use crate::errors::StarknetRpcResult;
use crate::Starknet;
use mp_block::MadaraMaybePreconfirmedBlockInfo;
use mp_rpc::v0_10_0::{
    BlockId, BlockStatus, BlockWithTxHashes, MaybePreConfirmedBlockWithTxHashes, PreConfirmedBlockWithTxHashes,
};

pub fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePreConfirmedBlockWithTxHashes> {
    let view = starknet.resolve_block_view(block_id)?;
    let block_info = view.get_block_info()?;

    let status = if view.is_preconfirmed() {
        BlockStatus::PreConfirmed
    } else if view.is_on_l1() {
        BlockStatus::AcceptedOnL1
    } else {
        BlockStatus::AcceptedOnL2
    };

    match block_info {
        MadaraMaybePreconfirmedBlockInfo::Preconfirmed(block) => {
            Ok(MaybePreConfirmedBlockWithTxHashes::PreConfirmed(PreConfirmedBlockWithTxHashes {
                transactions: block.tx_hashes,
                pre_confirmed_block_header: block.header.to_rpc_v0_9(),
            }))
        }
        MadaraMaybePreconfirmedBlockInfo::Confirmed(block) => {
            let tx_hashes = view.get_executed_transactions(..)?.into_iter().map(|tx| *tx.receipt.transaction_hash()).collect();
            Ok(MaybePreConfirmedBlockWithTxHashes::Block(BlockWithTxHashes {
                transactions: tx_hashes,
                status,
                block_header: block.to_rpc_v0_10(),
            }))
        }
    }
}
