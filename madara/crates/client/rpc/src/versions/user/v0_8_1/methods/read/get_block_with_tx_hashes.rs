use mp_block::MadaraMaybePendingBlockInfo;
use mp_rpc::v0_8_1::{
    BlockId, BlockStatus, BlockWithTxHashes, MaybePendingBlockWithTxHashes, PendingBlockWithTxHashes,
};

use crate::errors::StarknetRpcResult;
use crate::Starknet;

pub fn get_block_with_tx_hashes(
    starknet: &Starknet,
    block_id: BlockId,
) -> StarknetRpcResult<MaybePendingBlockWithTxHashes> {
    let block = starknet.get_block_info(&block_id)?;

    let block_txs_hashes = block.tx_hashes().to_vec();

    match block {
        MadaraMaybePendingBlockInfo::Pending(block_info) => {
            Ok(MaybePendingBlockWithTxHashes::Pending(PendingBlockWithTxHashes {
                transactions: block_txs_hashes,
                pending_block_header: block_info.into(),
            }))
        }
        MadaraMaybePendingBlockInfo::NotPending(block_info) => {
            let status = if block_info.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxHashes::Block(BlockWithTxHashes {
                transactions: block_txs_hashes,
                status,
                block_header: block_info.into(),
            }))
        }
    }
}
