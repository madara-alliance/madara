use dp_block::{BlockId, BlockTag};
use dp_convert::to_felt::ToFelt;
use jsonrpsee::core::RpcResult;
use starknet_core::types::{SyncStatus, SyncStatusType};

use crate::utils::ResultExt;
use crate::Starknet;

/// Returns an object about the sync status, or false if the node is not synching
///
/// ### Arguments
///
/// This function does not take any arguments.
///
/// ### Returns
///
/// * `Syncing` - An Enum that can either be a `dc_rpc_core::SyncStatus` struct representing the
///   sync status, or a `Boolean` (`false`) indicating that the node is not currently synchronizing.
///
/// This is an asynchronous function due to its reliance on `sync_service.best_seen_block()`,
/// which potentially involves network communication and processing to determine the best block
/// seen by the sync service.
pub async fn syncing(starknet: &Starknet) -> RpcResult<SyncStatusType> {
    // obtain best seen (highest) block number

    let current_block_info = match starknet
        .block_storage()
        .get_block_info(&BlockId::Tag(BlockTag::Latest))
        .or_internal_server_error("Error getting latest block")?
    {
        Some(block_info) => block_info,
        None => return Ok(SyncStatusType::NotSyncing),
    };
    let starting_block_num = starknet.starting_block;
    let starting_block_hash = starknet.get_block_info(BlockId::Number(starting_block_num))?.block_hash().to_felt();
    let current_block_num = current_block_info.block_n();
    let current_block_hash = current_block_info.block_hash().to_felt();

    Ok(SyncStatusType::Syncing(SyncStatus {
        starting_block_num,
        starting_block_hash,
        highest_block_num: current_block_num, // TODO(merge): is this correct,?
        highest_block_hash: current_block_hash,
        current_block_num,
        current_block_hash,
    }))
}
