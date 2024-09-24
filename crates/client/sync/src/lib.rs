use crate::l2::L2SyncConfig;
use anyhow::Context;
use fetch::fetchers::FetchConfig;
use mc_block_import::BlockImporter;
use mc_db::MadaraBackend;
use mc_telemetry::TelemetryHandle;
use mp_convert::ToFelt;
use starknet_providers::SequencerGatewayProvider;
use std::{sync::Arc, time::Duration};

pub mod fetch;
pub mod l2;
pub mod metrics;
pub mod tests;
pub mod utils;

#[allow(clippy::too_many_arguments)]
pub async fn sync(
    backend: &Arc<MadaraBackend>,
    block_importer: Arc<BlockImporter>,
    fetch_config: FetchConfig,
    starting_block: Option<u64>,
    backup_every_n_blocks: Option<u64>,
    telemetry: TelemetryHandle,
    pending_block_poll_interval: Duration,
) -> anyhow::Result<()> {
    let (starting_block, ignore_block_order) = if let Some(starting_block) = starting_block {
        log::warn!("Forcing unordered state. This will most probably break your database.");
        (starting_block, true)
    } else {
        (
            backend
                .get_block_n(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
                .context("getting sync tip")?
                .map(|block_id| block_id + 1) // next block after the tip
                .unwrap_or_default() as _, // or genesis
            false,
        )
    };

    log::info!("⛓️  Starting L2 sync from block {}", starting_block);

    let provider = SequencerGatewayProvider::new(
        fetch_config.gateway.clone(),
        fetch_config.feeder_gateway.clone(),
        fetch_config.chain_id.to_felt(),
    );
    let provider = match &fetch_config.api_key {
        Some(api_key) => provider.with_header("X-Throttling-Bypass".to_string(), api_key.clone()),
        None => provider,
    };

    l2::sync(
        backend,
        provider,
        L2SyncConfig {
            first_block: starting_block,
            n_blocks_to_sync: fetch_config.n_blocks_to_sync,
            verify: fetch_config.verify,
            sync_polling_interval: fetch_config.sync_polling_interval,
            backup_every_n_blocks,
            pending_block_poll_interval,
            ignore_block_order,
        },
        backend.chain_config().chain_id.clone(),
        telemetry,
        block_importer,
    )
    .await?;

    Ok(())
}
