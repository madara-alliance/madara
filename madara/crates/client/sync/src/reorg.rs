use anyhow::Context;
use mc_db::{db_block_id::RawDbBlockId, MadaraBackend};
use mc_gateway_client::GatewayProvider;
use mp_block::BlockId;
use mp_gateway::block::ProviderBlockPendingMaybe;

/// Detects if a reorganization has occurred by comparing local and gateway block hashes
///
/// Returns:
/// - Ok(None) if no reorg detected
/// - Ok(Some(common_ancestor)) if reorg detected
/// - Err if there's an error accessing data
pub async fn detect_reorg(
    block_n: u64,
    backend: &MadaraBackend,
    client: &GatewayProvider,
) -> anyhow::Result<Option<u64>> {
    if block_n == 0 {
        return Ok(None);
    }
    let gateway_block = client
        .get_block(BlockId::Number(block_n))
        .await
        .context("Failed to fetch block from gateway")?;

    let gateway_block = match gateway_block {
        ProviderBlockPendingMaybe::NonPending(block) => block,
        ProviderBlockPendingMaybe::Pending(_) => {
            anyhow::bail!("Expected non-pending block at height {}", block_n);
        }
    };

    let local_hash = backend.get_block_hash(&RawDbBlockId::Number(block_n))?;
    if let Some(local_hash) = local_hash {
        tracing::debug!(
            "Checking block #{}: local_hash={:#x}, gateway_hash={:#x}",
            block_n,
            local_hash,
            gateway_block.block_hash
        );
        if local_hash != gateway_block.block_hash {
            tracing::warn!(
                "⚠️ REORG DETECTED at block {}: local {:#x} != gateway {:#x}",
                block_n,
                local_hash,
                gateway_block.block_hash
            );

            let common_ancestor = find_common_ancestor(
                block_n,
                backend,
                client,
            ).await?;

            return Ok(Some(common_ancestor));
        } else {
            tracing::debug!("Block #{} hashes match - no reorg", block_n);
        }
    } else {
        tracing::debug!("Block #{} not found locally - skipping reorg check", block_n);
    }

    if block_n > 0 {
        let parent_n = block_n - 1;
        let local_parent_hash = backend.get_block_hash(&RawDbBlockId::Number(parent_n))?;

        if let Some(local_parent) = local_parent_hash {
            if local_parent != gateway_block.parent_block_hash {
                let common_ancestor = find_common_ancestor(
                    parent_n,
                    backend,
                    client,
                ).await?;

                tracing::warn!(
                    "⚠️ Parent hash mismatch at block {}: expected {} != actual {}",
                    block_n,
                    local_parent,
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
///
/// #####
///
/// 1. Starting from `start_block - 1`, walks backwards through the chain
/// 2. For each block, compares local hash with gateway hash
/// 3. Returns the first block where hashes match (common ancestor)
/// 4. If no match found all the way to genesis, returns 0
pub async fn find_common_ancestor(
    start_block: u64,
    backend: &MadaraBackend,
    client: &GatewayProvider,
) -> anyhow::Result<u64> {
    let mut check_block = start_block;

    tracing::info!("🔍 Finding common ancestor starting from block {}", start_block);

    while check_block > 0 {
        check_block -= 1;

        let local_hash = backend.get_block_hash(&RawDbBlockId::Number(check_block))?;
        if local_hash.is_none() {
            // Block not found locally - continue searching backwards
            // This can occur when the local node hasn't synced this far yet
            continue;
        }

        let gateway_block = client
            .get_block(BlockId::Number(check_block))
            .await
            .context("Failed to fetch block for common ancestor search")?;

        let gateway_block = match gateway_block {
            ProviderBlockPendingMaybe::NonPending(block) => block,
            ProviderBlockPendingMaybe::Pending(_) => continue,
        };

        if local_hash.unwrap() == gateway_block.block_hash {
            tracing::info!("✅ Found common ancestor at block {}", check_block);
            return Ok(check_block);
        }
    }
    tracing::warn!("⚠️ Reorg extends all the way to genesis!");
    Ok(0)
}
