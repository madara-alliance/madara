use mp_block::{BlockId, BlockTag};
use mp_rpc::{BlockHash, SyncStatus, SyncingStatus};
use starknet_types_core::felt::Felt;

use crate::errors::StarknetRpcResult;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;

const SYNC_THRESHOLD_BLOCKS: u64 = 6;

/// Returns an object about the sync status, or false if the node is not syncing
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `Syncing` - An Enum that can either be a `mc_rpc_core::SyncStatus` struct representing the
///   sync status, or a `Boolean` (`false`) indicating that the node is not currently synchronizing.
///
/// Following the spec: https://github.com/starkware-libs/starknet-specs/blob/2030a650be4e40cfa34d5051a0334f375384a421/api/starknet_api_openrpc.json#L765
/// if the node is synced it will return a SyncingStatus::NotSyncing which is a boolean false, and in case of syncing it will return a SyncStatus struct
pub async fn syncing(starknet: &Starknet) -> StarknetRpcResult<SyncingStatus> {
    // Get current block info first
    let (current_block_num, current_block_hash) = match starknet
        .backend
        .get_block_info(&BlockId::Tag(BlockTag::Latest))
        .or_internal_server_error("Error getting latest block")?
        .ok_or_internal_server_error("Latest block not found")
    {
        Ok(block_info) => {
            let current_block_info =
                block_info.as_nonpending().ok_or_internal_server_error("Latest block cannot be pending")?;
            let current_block_num = current_block_info.header.block_number;
            let current_block_hash = current_block_info.block_hash;
            (current_block_num, current_block_hash)
        }
        Err(_err) => (0u64, Felt::ZERO),
    };

    // Get the sync status from the provider with retry logic
    let sync_status =
        starknet.sync_status().await.or_internal_server_error("Error getting sync status after retries")?;

    // Get the starting block number from the sync_status
    let starting_block_num = starknet
        .backend
        .get_starting_block_info()?
        .starting_block_num
        .ok_or_internal_server_error("Error getting starting block")?;

    // Get the starting block hash - if it's not in sync status or is zero, fetch it from the backend
    let starting_block_hash = if sync_status.starting_block_hash == BlockHash::default() {
        // We need to fetch the starting block hash from the backend
        let starting_block_info = starknet
            .backend
            .get_block_info(&BlockId::Number(starting_block_num))
            .or_internal_server_error("Error getting starting block")?
            .ok_or_internal_server_error(format!("Starting block not found: block number {}", starting_block_num))?;

        let starting_block_info =
            starting_block_info.as_nonpending().ok_or_internal_server_error("Starting block cannot be pending")?;

        starting_block_info.block_hash
    } else {
        sync_status.starting_block_hash
    };

    // Get the highest block info from the sync status
    let highest_block_num = sync_status.highest_block_num;
    let highest_block_hash = sync_status.highest_block_hash;

    // If the highest block number is 0 or less than current, we're not syncing
    if highest_block_num == 0 || highest_block_num - SYNC_THRESHOLD_BLOCKS <= current_block_num {
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
