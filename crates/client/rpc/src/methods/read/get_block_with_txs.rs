use starknet_core::types::{BlockId, MaybePendingBlockWithTxs};

use crate::errors::StarknetRpcResult;
use crate::methods::get_block;
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
pub fn get_block_with_txs(starknet: &Starknet, block_id: BlockId) -> StarknetRpcResult<MaybePendingBlockWithTxs> {
    get_block::get_block_with_txs(starknet, &block_id.into())
}
