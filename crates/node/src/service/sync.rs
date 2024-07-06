use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use dc_db::{DatabaseService, DeoxysBackend};
use dc_metrics::MetricsRegistry;
use dc_sync::fetch::fetchers::FetchConfig;
use dc_sync::metrics::block_metrics::BlockMetrics;
use dc_sync::utility::l1_free_rpc_get;
use dc_telemetry::TelemetryHandle;
use primitive_types::H160;
use starknet_types_core::felt::Felt;
use tokio::task::JoinSet;
use url::Url;

use crate::cli::SyncParams;

#[derive(Clone)]
pub struct SyncService {
    db_backend: Arc<DeoxysBackend>,
    base_path: PathBuf,
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    l1_endpoint: Option<Url>,
    l1_core_address: H160,
    starting_block: Option<u64>,
    block_metrics: BlockMetrics,
    chain_id: Felt,
    start_params: Option<TelemetryHandle>,
    disabled: bool,
    pending_block_poll_interval: Duration,
}

impl SyncService {
    pub async fn new(
        config: &SyncParams,
        db: &DatabaseService,
        metrics_handle: MetricsRegistry,
        telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        let block_metrics = BlockMetrics::register(&metrics_handle)?;
        let fetch_config = config.block_fetch_config();

        let l1_endpoint = if !config.sync_l1_disabled {
            if let Some(l1_rpc_url) = &config.l1_endpoint {
                Some(l1_rpc_url.clone())
            } else {
                let l1_rpc_url = l1_free_rpc_get().await.expect("finding the best RPC URL");
                Some(Url::parse(l1_rpc_url).expect("parsing the RPC URL"))
            }
        } else {
            None
        };

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            base_path: db.get_base_path(),
            fetch_config,
            l1_endpoint,
            l1_core_address: config.network.l1_core_address(),
            starting_block: config.starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_metrics,
            chain_id: config.network.chain_id(),
            start_params: Some(telemetry),
            disabled: config.sync_disabled,
            pending_block_poll_interval: Duration::from_secs(config.pending_block_poll_interval),
        })
    }
    pub async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let SyncService {
            fetch_config,
            backup_every_n_blocks,
            l1_endpoint,
            l1_core_address,
            starting_block,
            block_metrics,
            chain_id,
            pending_block_poll_interval,
            ..
        } = self.clone();
        let telemetry = self.start_params.take().context("service already started")?;

        let db_backend = Arc::clone(&self.db_backend);
        let base_path = self.base_path.to_path_buf();
        join_set.spawn(async move {
            dc_sync::starknet_sync_worker::sync(
                &db_backend,
                base_path,
                fetch_config,
                l1_endpoint,
                l1_core_address,
                starting_block,
                backup_every_n_blocks,
                block_metrics,
                chain_id,
                telemetry,
                pending_block_poll_interval,
            )
            .await
        });

        Ok(())
    }
}
