use mp_block::{BlockId, BlockTag};
use mp_rpc::{BlockHash, SyncStatus, SyncingStatus};

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
    // Get the sync status from the provider
    let sync_status = starknet.sync_status().await.or_internal_server_error("Error getting sync status")?;

    // Get current block info
    let Some(current_block_info) = starknet
        .backend
        .get_block_info(&BlockId::Tag(BlockTag::Latest))
        .or_internal_server_error("Error getting latest block")?
    else {
        return Ok(SyncingStatus::NotSyncing);
    };

    let current_block_info =
        current_block_info.as_nonpending().ok_or_internal_server_error("Latest block cannot be pending")?;
    let current_block_num = current_block_info.header.block_number;
    let current_block_hash = current_block_info.block_hash;

    // Get the starting block number from sync status
    let starting_block_num = sync_status.starting_block_num;

    // Get the starting block hash - if it's not in sync status or is zero, fetch it from the backend
    let starting_block_hash = if sync_status.starting_block_hash == BlockHash::default() {
        // We need to fetch the starting block hash from the backend
        let Some(starting_block_info) = starknet
            .backend
            .get_block_info(&BlockId::Number(starting_block_num))
            .or_internal_server_error("Error getting starting block")?
        else {
            // If we can't get the starting block info, use a default hash
            // This is a fallback and shouldn't normally happen
            tracing::warn!("Could not get starting block info for block {}", starting_block_num);
            return Ok(SyncingStatus::NotSyncing);
        };

        let starting_block_info =
            starting_block_info.as_nonpending().ok_or_internal_server_error("Starting block cannot be pending")?;

        starting_block_info.block_hash
    } else {
        // Use the hash from sync status
        sync_status.starting_block_hash
    };

    // Get highest block info from sync status
    let highest_block_num = sync_status.highest_block_num;
    let highest_block_hash = sync_status.highest_block_hash;

    // If highest block number is 0 or less than current, we're not syncing
    if highest_block_num == 0 || highest_block_num - 6 <= current_block_num {
        return Ok(SyncingStatus::NotSyncing);
    }

    Ok(SyncingStatus::Syncing(SyncStatus {
        starting_block_num,
        starting_block_hash,
        highest_block_num,
        highest_block_hash,
        current_block_num,
        current_block_hash,
    }))
}
