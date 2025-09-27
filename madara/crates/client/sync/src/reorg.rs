use anyhow::Context;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mp_block::BlockId;
use std::sync::Arc;

/// Detects if a reorganization has occurred by comparing local and gateway block hashes
///
/// Returns:
/// - Ok(None) if no reorg detected
/// - Ok(Some(common_ancestor)) if reorg detected
/// - Err if there's an error accessing data
pub async fn detect_reorg(
    block_n: u64,
    backend: &Arc<MadaraBackend>,
    client: &Arc<GatewayProvider>,
) -> anyhow::Result<Option<u64>> {
    tracing::info!("🔎 detect_reorg called with block_n: {}, latest_local: {:?}", 
                  block_n, backend.latest_confirmed_block_n());
    
    if block_n == 0 {
        return Ok(None);
    }
    
    // Determine which block to actually check
    let block_to_check = {
        let latest_local = backend.latest_confirmed_block_n();
        if let Some(latest) = latest_local {
            if block_n > latest {
                tracing::debug!("Block #{} beyond local chain (latest: {}), checking latest block for reorg", block_n, latest);
                latest
            } else {
                block_n
            }
        } else {
            // No local blocks yet
            tracing::info!("❌ No local blocks found, skipping reorg check");
            return Ok(None);
        }
    };
    
    let gateway_block = client
        .get_block(BlockId::Number(block_to_check))
        .await
        .context("Failed to fetch block from gateway")?;

    // Get local block to compare
    let local_block_view = backend.block_view_on_confirmed(block_to_check);
    
    if let Some(local_view) = local_block_view {
        let local_info = local_view.get_block_info()?;
        let local_hash = local_info.block_hash;
        tracing::debug!(
            "Checking block #{}: local_hash={:#x}, gateway_hash={:#x}",
            block_to_check,
            local_hash,
            gateway_block.block_hash
        );
        
        if local_hash != gateway_block.block_hash {
            tracing::warn!(
                "⚠️ REORG DETECTED at block {}: local {:#x} != gateway {:#x}",
                block_to_check,
                local_hash,
                gateway_block.block_hash
            );

            let common_ancestor = find_common_ancestor(
                block_to_check,
                backend,
                client,
            ).await?;

            return Ok(Some(common_ancestor));
        } else {
            tracing::debug!("Block #{} hashes match - no reorg", block_to_check);
        }
    } else {
        tracing::debug!("Block #{} not found locally - no reorg check possible", block_to_check);
    }

    // Check parent hash as well
    if block_to_check > 0 {
        let parent_n = block_to_check - 1;
        let local_parent_view = backend.block_view_on_confirmed(parent_n);

        if let Some(local_parent) = local_parent_view {
            let local_parent_info = local_parent.get_block_info()?;
            let local_parent_hash = local_parent_info.block_hash;
            if local_parent_hash != gateway_block.parent_block_hash {
                let common_ancestor = find_common_ancestor(
                    parent_n,
                    backend,
                    client,
                ).await?;

                tracing::warn!(
                    "⚠️ Parent hash mismatch at block {}: expected {} != actual {}",
                    block_to_check,
                    local_parent_hash,
                    gateway_block.parent_block_hash
                );

                return Ok(Some(common_ancestor));
            }
        }
    }

    Ok(None)
}

/// Finds the common ancestor block between the local chain and gateway chain during a reorg
///
/// This function walks backwards from a given starting block, comparing block hashes
/// between the local database and the gateway provider until it finds a block where
/// both chains agree (same hash). This block becomes the common ancestor from which
/// the chains diverged.
///
/// # Arguments
///
/// * `start_block` - The block number to start searching backwards from (typically where reorg was detected)
/// * `backend` - Reference to the local Madara backend for accessing local blockchain data
/// * `client` - Reference to the gateway provider for fetching remote chain data
///
/// # Returns
///
/// * `Ok(u64)` - The block number of the common ancestor
/// * `Err` - If there's an error accessing local or remote blockchain data
pub async fn find_common_ancestor(
    start_block: u64,
    backend: &Arc<MadaraBackend>,
    client: &Arc<GatewayProvider>,
) -> anyhow::Result<u64> {
    let mut check_block = start_block;

    tracing::info!("🔍 Finding common ancestor starting from block {}", start_block);

    while check_block > 0 {
        check_block -= 1;

        let local_block_view = backend.block_view_on_confirmed(check_block);
        if local_block_view.is_none() {
            // Block not found locally - continue searching backwards
            // This can occur when the local node hasn't synced this far yet
            continue;
        }

        let gateway_block = client
            .get_block(BlockId::Number(check_block))
            .await
            .context("Failed to fetch block for common ancestor search")?;

        let local_info = local_block_view.unwrap().get_block_info()?;
        let local_hash = local_info.block_hash;
        
        if local_hash == gateway_block.block_hash {
            tracing::info!("✅ Found common ancestor at block {}", check_block);
            return Ok(check_block);
        }
    }
    
    tracing::warn!("⚠️ Reorg extends all the way to genesis!");
    Ok(0)
}