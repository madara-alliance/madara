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
    // Skip genesis block
    if block_n == 0 {
        return Ok(None);
    }

    // Fetch block header from gateway
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

    // Check if we have this block locally
    let local_hash = backend.get_block_hash(&RawDbBlockId::Number(block_n))?;

    if let Some(local_hash) = local_hash {
        // Log the comparison
        tracing::info!(
            "Checking block #{}: local_hash={:#x}, gateway_hash={:#x}",
            block_n,
            local_hash,
            gateway_block.block_hash
        );

        // We have a block at this height - check if it matches
        if local_hash != gateway_block.block_hash {
            // Reorg detected! Find common ancestor
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

    // Also check parent hash consistency if we have the parent
    if block_n > 0 {
        let parent_n = block_n - 1;
        let local_parent_hash = backend.get_block_hash(&RawDbBlockId::Number(parent_n))?;

        if let Some(local_parent) = local_parent_hash {
            if local_parent != gateway_block.parent_block_hash {
                // Parent mismatch - this is also a reorg
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

/// Finds the common ancestor between local chain and gateway chain
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
            // We don't have this block locally
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
            // Found common ancestor!
            tracing::info!("✅ Found common ancestor at block {}", check_block);
            return Ok(check_block);
        }
    }

    // If we get here, reorg goes all the way to genesis
    tracing::warn!("⚠️ Reorg extends all the way to genesis!");
    Ok(0)
}
