use mc_gateway_client::GatewayProvider;
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
    let current_block_num = current_block_info.header.block_number;
    let current_block_hash = current_block_info.block_hash;

    // Get the starting block (the one the node started syncing from)
    let starting_block_num = 0; // We'll use 0 as the starting block for now
    let starting_block_info = starknet.get_block_info(&BlockId::Number(starting_block_num))?;
    let starting_block_info =
        starting_block_info.as_nonpending().ok_or_internal_server_error("Block cannot be pending")?;
    let starting_block_hash = starting_block_info.block_hash;

    // Create a GatewayProvider to fetch the latest block from the network
    let chain_config = starknet.clone_chain_config();
    let gateway_url = chain_config.gateway_url.clone();
    let feeder_gateway_url = chain_config.feeder_gateway_url.clone();
    
    let provider = GatewayProvider::new(gateway_url, feeder_gateway_url);
    
    // Fetch the latest block from the network
    let network_latest_block = provider
        .get_block(BlockId::Tag(BlockTag::Latest))
        .await
        .or_internal_server_error("Error fetching latest block from network")?;
    
    let network_latest_block = network_latest_block
        .non_pending()
        .ok_or_internal_server_error("Latest block from network cannot be pending")?;
    
    let highest_block_num = network_latest_block.block_number;
    let highest_block_hash = network_latest_block.block_hash;

    // If we're not syncing or we've caught up with the network, return NotSyncing
    if highest_block_num <= current_block_num {
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
