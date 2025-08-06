use mp_block::MadaraMaybePendingBlockInfo;
use mp_rpc::v0_8_1::{BlockId, BlockStatus, BlockWithTxs, MaybePendingBlockWithTxs, PendingBlockWithTxs, TxnWithHash};

use crate::errors::StarknetRpcResult;
use crate::Starknet;

pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<MaybePendingBlockWithTxs> {
    let block = starknet.get_block(&block_id)?;

    let transactions_with_hash = Iterator::zip(block.inner.transactions.into_iter(), block.info.tx_hashes())
        .map(|(transaction, hash)| TxnWithHash { transaction: transaction.into(), transaction_hash: *hash })
        .collect();

    match block.info {
        MadaraMaybePendingBlockInfo::Pending(block_info) => {
            Ok(MaybePendingBlockWithTxs::Pending(PendingBlockWithTxs {
                transactions: transactions_with_hash,
                pending_block_header: block_info.into(),
            }))
        }
        MadaraMaybePendingBlockInfo::NotPending(block_info) => {
            let status = if block_info.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxs::Block(BlockWithTxs {
                transactions: transactions_with_hash,
                status,
                block_header: block_info.into(),
            }))
        }
    }
}
