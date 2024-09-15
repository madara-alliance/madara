use crate::cli::{NetworkType, SyncParams};
use anyhow::Context;
use mc_db::db_metrics::DbMetrics;
use mc_db::{DatabaseService, MadaraBackend};
use mc_metrics::MetricsRegistry;
use mc_sync::fetch::fetchers::FetchConfig;
use mc_sync::metrics::block_metrics::BlockMetrics;
use mc_telemetry::TelemetryHandle;
use mp_chain_config::ChainConfig;
use mp_utils::service::{Service, TaskGroup};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct SyncService {
    db_backend: Arc<MadaraBackend>,
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    starting_block: Option<u64>,
    block_metrics: BlockMetrics,
    db_metrics: DbMetrics,
    start_params: Option<TelemetryHandle>,
    disabled: bool,
    pending_block_poll_interval: Duration,
}

impl SyncService {
    pub async fn new(
        config: &SyncParams,
        chain_config: Arc<ChainConfig>,
        network: NetworkType,
        db: &DatabaseService,
        metrics_handle: MetricsRegistry,
        telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        let block_metrics = BlockMetrics::register(&metrics_handle)?;
        let db_metrics = DbMetrics::register(&metrics_handle)?;
        let fetch_config = config.block_fetch_config(chain_config.chain_id.clone(), network);

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            fetch_config,
            starting_block: config.unsafe_starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_metrics,
            db_metrics,
            start_params: Some(telemetry),
            disabled: config.sync_disabled,
            pending_block_poll_interval: Duration::from_secs(config.pending_block_poll_interval),
        })
    }
}

#[async_trait::async_trait]
impl Service for SyncService {
    fn name(&self) -> &str {
        "L2 Sync"
    }
    async fn start(&mut self, join_set: &mut TaskGroup) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let SyncService {
            fetch_config,
            backup_every_n_blocks,
            starting_block,
            block_metrics,
            db_metrics,
            pending_block_poll_interval,
            ..
        } = self.clone();
        let telemetry = self.start_params.take().context("Service already started")?;

        let db_backend = Arc::clone(&self.db_backend);

        join_set.spawn(async move {
            mc_sync::sync(
                &db_backend,
                fetch_config,
                starting_block,
                backup_every_n_blocks,
                block_metrics,
                db_metrics,
                telemetry,
                pending_block_poll_interval,
            )
            .await
        });

        Ok(())
    }
}
