use crate::errors::StarknetRpcResult;
use crate::Starknet;
use anyhow::Context;
use mp_rpc::{SyncStatus, SyncingStatus};
use starknet_types_core::felt::Felt;

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
pub fn syncing(starknet: &Starknet) -> StarknetRpcResult<SyncingStatus> {
    use mc_db::sync_status::SyncStatus as BackendSyncStatus;

    let BackendSyncStatus::Running { highest_block_n, highest_block_hash } = starknet.backend.get_sync_status() else {
        return Ok(SyncingStatus::NotSyncing);
    };

    let current_block = starknet.backend.block_view_on_last_confirmed();

    // If the current block number is within the sync threshold, return NotSyncing. Else return Syncing with info
    if current_block.as_ref().is_none_or(|view| highest_block_n <= view.block_number() + SYNC_THRESHOLD_BLOCKS) {
        return Ok(SyncingStatus::NotSyncing);
    }

    let (current_block_num, current_block_hash) = if let Some(view) = current_block {
        (view.block_number(), view.get_block_info()?.block_hash)
    } else {
        // No blocks in backend yet.
        (0u64, Felt::ZERO)
    };

    let (starting_block_num, starting_block_hash) =
        if let Some(starting_block_num) = starknet.backend.get_starting_block() {
            let view = starknet
                .backend
                .block_view_on_confirmed(starting_block_num)
                .context("Starting block is not in the backend")?;
            (view.block_number(), view.get_block_info()?.block_hash)
        } else {
            // No starting block.
            (0u64, Felt::ZERO)
        };

    Ok(SyncingStatus::Syncing(SyncStatus {
        starting_block_num,
        starting_block_hash,
        highest_block_num: highest_block_n,
        highest_block_hash,
        current_block_num,
        current_block_hash,
    }))
}
