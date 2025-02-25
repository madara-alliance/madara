use mp_block::{BlockId, BlockTag};
use mp_rpc::{SyncStatus, SyncingStatus};

use crate::errors::StarknetRpcResult;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

/// Returns an object about the sync status, or false if the node is not synching
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `Syncing` - An Enum that can either be a `mc_rpc_core::SyncStatus` struct representing the
///   sync status, or a `Boolean` (`false`) indicating that the node is not currently synchronizing.
pub async fn syncing(starknet: &Starknet) -> StarknetRpcResult<SyncingStatus> {
    // obtain best seen (highest) block number
    let Some(current_block_info) = starknet
        .backend
        .get_block_info(&BlockId::Tag(BlockTag::Latest))
        .or_internal_server_error("Error getting latest block")?
    else {
        return Ok(SyncingStatus::NotSyncing); // TODO: This doesn't really make sense? This can only happen when there are no block in the db at all.
    };

    let current_block_info =
        current_block_info.as_nonpending().ok_or_internal_server_error("Latest block cannot be pending")?;
    let starting_block_num = 0; // TODO(rpc): fix this // starknet.starting_block;
    let starting_block_info = starknet.get_block_info(&BlockId::Number(starting_block_num))?;
    let starting_block_info =
        starting_block_info.as_nonpending().ok_or_internal_server_error("Block cannot be pending")?;
    let starting_block_hash = starting_block_info.block_hash;
    let current_block_num = current_block_info.header.block_number;
    let current_block_hash = current_block_info.block_hash;

    Ok(SyncingStatus::Syncing(SyncStatus {
        starting_block_num,
        starting_block_hash,
        highest_block_num: current_block_num, // TODO(merge): is this correct?
        highest_block_hash: current_block_hash,
        current_block_num,
        current_block_hash,
    }))
}
