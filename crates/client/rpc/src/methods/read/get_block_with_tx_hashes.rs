use jsonrpsee::core::RpcResult;
use starknet_core::types::{BlockId, MaybePendingBlockWithTxHashes};

use crate::methods::get_block;
use crate::Starknet;

/// Get block information with transaction hashes given the block id.
///
/// ### Arguments
///
/// * `block_id` - The hash of the requested block, or number (height) of the requested block, or a
///   block tag.
///
/// ### Returns
///
/// Returns block information with transaction hashes. This includes either a confirmed block or
/// a pending block with transaction hashes, depending on the state of the requested block.
/// In case the block is not found, returns a `StarknetRpcApiError` with `BlockNotFound`.
pub fn get_block_with_tx_hashes(starknet: &Starknet, block_id: BlockId) -> RpcResult<MaybePendingBlockWithTxHashes> {
    get_block::get_block_with_tx_hashes(starknet, &block_id.into())
}
