use mp_block::{BlockId, BlockTag};
use mp_rpc::{SyncStatus, SyncingStatus};
use starknet_types_core::felt::Felt;

use crate::errors::StarknetRpcResult;
use crate::utils::{OptionExt, ResultExt};
use crate::Starknet;
use mc_db::SyncStatus as MadaraSyncStatus;

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
                block_info.as_closed().ok_or_internal_server_error("Latest block cannot be pending")?;
            let current_block_num = current_block_info.header.block_number;
            let current_block_hash = current_block_info.block_hash;
            (current_block_num, current_block_hash)
        }
        Err(_err) => (0u64, Felt::ZERO),
    };

    // Get the highest block number and hash from MadaraBackend
    let highest_block_info = starknet.backend.get_sync_status().await;
    // Check if the node is syncing or not
    match highest_block_info {
        MadaraSyncStatus::Running { highest_block_hash, highest_block_n } => {
            // Get the starting block number from MadaraBackend
            let starting_block_n = starknet.backend.get_starting_block();
            // Get the starting block hash from MadaraBackend
            let (starting_block_n, starting_block_hash) = match starting_block_n {
                Some(starting_block_n) => {
                    let starting_block_info = starknet
                        .backend
                        .get_block_info(&BlockId::Number(starting_block_n))
                        .or_internal_server_error("Error getting starting block")?
                        .ok_or_internal_server_error(format!(
                            "Starting block not found: block number {}",
                            starting_block_n
                        ))?;
                    let starting_block_info = starting_block_info
                        .as_closed()
                        .ok_or_internal_server_error("Starting block cannot be pending")?;
                    (starting_block_n, starting_block_info.block_hash)
                }
                None => {
                    // According to spec, if the starting block is not set in DB, we should return 0 and the empty hash
                    (0, Felt::default())
                }
            };
            // If the current block number is within the sync threshold, return NotSyncing. Else return Syncing with info
            if highest_block_n <= current_block_num + SYNC_THRESHOLD_BLOCKS {
                Ok(SyncingStatus::NotSyncing)
            } else {
                Ok(SyncingStatus::Syncing(SyncStatus {
                    starting_block_num: starting_block_n,
                    starting_block_hash,
                    highest_block_num: highest_block_n,
                    highest_block_hash,
                    current_block_num,
                    current_block_hash,
                }))
            }
        }
        // If the node is not syncing, return NotSyncing
        MadaraSyncStatus::NotRunning => Ok(SyncingStatus::NotSyncing),
    }
}
