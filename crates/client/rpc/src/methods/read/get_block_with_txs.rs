use starknet_core::types::{BlockId, MaybePendingBlockWithTxs};

use jsonrpsee::core::RpcResult;
use mp_block::MadaraMaybePendingBlockInfo;
use starknet_core::types::{BlockStatus, BlockWithTxs, PendingBlockWithTxs};

use crate::Starknet;

/// Get block information with full transactions given the block id.
///
/// This function retrieves detailed information about a specific block in the StarkNet network,
/// including all transactions contained within that block. The block is identified using its
/// unique block id, which can be the block's hash, its number (height), or a block tag.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag. This parameter is used to specify the block from which to retrieve information and
///   transactions.
///
/// ### Returns
///
/// Returns detailed block information along with full transactions. Depending on the state of
/// the block, this can include either a confirmed block or a pending block with its
/// transactions. In case the specified block is not found, returns a `StarknetRpcApiError` with
/// `BlockNotFound`.
pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxs> {
    let block = starknet.get_block(&block_id)?;

    let transactions_core = Iterator::zip(block.inner.transactions.iter(), block.info.tx_hashes())
        .map(|(transaction, hash)| transaction.clone().to_core(*hash))
        .collect();

    match block.info {
        MadaraMaybePendingBlockInfo::Pending(block) => {
            Ok(MaybePendingBlockWithTxs::PendingBlock(PendingBlockWithTxs {
                transactions: transactions_core,
                parent_hash: block.header.parent_block_hash,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
            }))
        }
        MadaraMaybePendingBlockInfo::NotPending(block) => {
            let status = if block.header.block_number <= starknet.get_l1_last_confirmed_block()? {
                BlockStatus::AcceptedOnL1
            } else {
                BlockStatus::AcceptedOnL2
            };
            Ok(MaybePendingBlockWithTxs::Block(BlockWithTxs {
                status,
                block_hash: block.block_hash,
                parent_hash: block.header.parent_block_hash,
                block_number: block.header.block_number,
                new_root: block.header.global_state_root,
                timestamp: block.header.block_timestamp,
                sequencer_address: block.header.sequencer_address,
                l1_gas_price: block.header.l1_gas_price.l1_gas_price(),
                l1_data_gas_price: block.header.l1_gas_price.l1_data_gas_price(),
                l1_da_mode: block.header.l1_da_mode.into(),
                starknet_version: block.header.protocol_version.to_string(),
                transactions: transactions_core,
            }))
        }
    }
}
