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
///
/// Following the spec: https://github.com/starkware-libs/starknet-specs/blob/2030a650be4e40cfa34d5051a0334f375384a421/api/starknet_api_openrpc.json#L765
/// if the node is synced it will return a SyncingStatus::NotSyncing which is a boolean false and in case of syncing it will return a SyncStatus struct
pub async fn syncing(starknet: &Starknet) -> StarknetRpcResult<SyncingStatus> {
    // Get the sync status from the provider with retry logic
    let sync_status = get_sync_status_with_retry(starknet)
        .await
        .or_internal_server_error("Error getting sync status after retries")?;

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

// Helper function to get sync status with exponential backoff retry
async fn get_sync_status_with_retry(starknet: &Starknet) -> Result<mp_rpc::SyncStatus, anyhow::Error> {
    use std::time::Duration;
    use tokio::time;

    const MAX_RETRIES: u32 = 3;
    const INITIAL_TIMEOUT_MS: u64 = 1000;

    let mut retries = 0;
    let mut timeout_ms = INITIAL_TIMEOUT_MS;

    loop {
        match time::timeout(Duration::from_millis(timeout_ms), starknet.sync_status()).await {
            // Successful response within timeout
            Ok(Ok(status)) => return Ok(status),

            // Timeout occurred
            Err(_) => {
                tracing::warn!("Timeout getting sync status after {}ms", timeout_ms);
            }

            // Error occurred within timeout
            Ok(Err(err)) => {
                tracing::warn!("Error getting sync status: {}", err);
            }
        }

        // Check if we've reached the maximum number of retries
        retries += 1;
        if retries >= MAX_RETRIES {
            return Err(anyhow::anyhow!("Failed to get sync status after {} retries", MAX_RETRIES));
        }

        // Exponential backoff
        timeout_ms *= 2;
        tracing::debug!("Retrying sync status (attempt {}/{}), timeout: {}ms", retries + 1, MAX_RETRIES, timeout_ms);

        // Add a small delay before retrying
        time::sleep(Duration::from_millis(100)).await;
    }
}
