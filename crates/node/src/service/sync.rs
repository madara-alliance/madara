use anyhow::Context;
use mc_metrics::MetricsRegistry;
use mc_sync::fetch::fetchers::FetchConfig;
use mc_sync::metrics::block_metrics::BlockMetrics;
use mc_sync::utility::l1_free_rpc_get;
use mc_telemetry::TelemetryHandle;
use primitive_types::H160;
use starknet_core::types::FieldElement;
use tokio::task::JoinSet;
use url::Url;

use crate::cli::SyncParams;

#[derive(Clone)]
pub struct SyncService {
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    l1_endpoint: Url,
    l1_core_address: H160,
    starting_block: Option<u64>,
    block_metrics: BlockMetrics,
    chain_id: FieldElement,
    start_params: Option<TelemetryHandle>,
}

impl SyncService {
    pub async fn new(
        config: &SyncParams,
        metrics_handle: MetricsRegistry,
        telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        let block_metrics = BlockMetrics::register(&metrics_handle)?;
        let l1_endpoint = if let Some(l1_rpc_url) = &config.l1_endpoint {
            l1_rpc_url.clone()
        } else {
            let l1_rpc_url = l1_free_rpc_get().await.expect("finding the best RPC URL");
            Url::parse(l1_rpc_url).expect("parsing the RPC URL")
        };
        Ok(Self {
            fetch_config: config.block_fetch_config(),
            l1_endpoint,
            l1_core_address: config.network.l1_core_address(),
            starting_block: config.starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_metrics,
            chain_id: config.network.chain_id(),
            start_params: Some(telemetry),
        })
    }
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        let SyncService {
            fetch_config,
            backup_every_n_blocks,
            l1_endpoint,
            l1_core_address,
            starting_block,
            block_metrics,
            chain_id,
            ..
        } = self.clone();
        let telemetry = self.start_params.take().context("service already started")?;

        join_set.spawn(async move {
            mc_sync::starknet_sync_worker::sync(
                fetch_config,
                l1_endpoint,
                l1_core_address,
                starting_block,
                backup_every_n_blocks,
                block_metrics,
                chain_id,
                telemetry,
            )
            .await
        });

        Ok(())
    }
}
