use crate::l2::L2SyncConfig;
use anyhow::Context;
use fetch::fetchers::FetchConfig;
use hyper::header::{HeaderName, HeaderValue};
use mc_block_import::BlockImporter;
use mc_db::MadaraBackend;
use mc_gateway_client::GatewayProvider;
use mc_telemetry::TelemetryHandle;
use mp_block::{BlockId, BlockTag};
use mp_sync::SyncStatusProvider;
use mp_utils::service::ServiceContext;
use std::{sync::Arc, time::Duration};

pub mod fetch;
pub mod l2;
pub mod metrics;
#[cfg(test)]
pub mod tests;

pub struct SyncConfig {
    pub block_importer: Arc<BlockImporter>,
    pub starting_block: Option<u64>,
    pub backup_every_n_blocks: Option<u64>,
    pub telemetry: Arc<TelemetryHandle>,
    pub pending_block_poll_interval: Duration,
}

#[tracing::instrument(skip(backend, ctx, fetch_config, sync_config, sync_status_provider))]
pub async fn l2_sync_worker(
    backend: Arc<MadaraBackend>,
    ctx: ServiceContext,
    fetch_config: FetchConfig,
    sync_config: SyncConfig,
    sync_status_provider: SyncStatusProvider,
) -> anyhow::Result<()> {
    let (starting_block, ignore_block_order) = if let Some(starting_block) = sync_config.starting_block {
        tracing::warn!("Forcing unordered state. This will most probably break your database.");
        (starting_block, true)
    } else {
        (
            backend
                .get_block_n(&BlockId::Tag(BlockTag::Latest))
                .context("getting sync tip")?
                .map(|block_id| block_id + 1) // next block after the tip
                .unwrap_or_default() as _, // or genesis
            false,
        )
    };
    // update the sync status with the starting block info
    sync_status_provider.set_starting_block_num(starting_block).await;
    tracing::info!("⛓️  Starting L2 sync from block {}", starting_block);

    let mut provider = GatewayProvider::new(fetch_config.gateway, fetch_config.feeder_gateway);
    if let Some(api_key) = fetch_config.api_key {
        provider.add_header(
            HeaderName::from_static("x-throttling-bypass"),
            HeaderValue::from_str(&api_key).with_context(|| "Invalid API key format")?,
        )
    }

    let l2_config = L2SyncConfig {
        first_block: starting_block,
        n_blocks_to_sync: fetch_config.n_blocks_to_sync,
        stop_on_sync: fetch_config.stop_on_sync,
        verify: fetch_config.verify,
        sync_polling_interval: fetch_config.sync_polling_interval,
        backup_every_n_blocks: sync_config.backup_every_n_blocks,
        flush_every_n_blocks: fetch_config.flush_every_n_blocks,
        flush_every_n_seconds: fetch_config.flush_every_n_seconds,
        pending_block_poll_interval: sync_config.pending_block_poll_interval,
        ignore_block_order,
        sync_parallelism: fetch_config.sync_parallelism,
        chain_id: backend.chain_config().chain_id.clone(),
        telemetry: sync_config.telemetry,
        block_importer: sync_config.block_importer,
        warp_update: fetch_config.warp_update,
    };

    l2::sync(backend, provider, ctx, l2_config, sync_status_provider).await?;

    Ok(())
}
