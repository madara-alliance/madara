use crate::cli::SyncParams;
use anyhow::Context;
use mc_block_import::BlockImporter;
use mc_db::{DatabaseService, MadaraBackend};
use mc_sync::fetch::fetchers::FetchConfig;
use mc_sync::SyncConfig;
use mc_telemetry::TelemetryHandle;
use mp_chain_config::ChainConfig;
use mp_utils::service::{MadaraService, Service, ServiceContext};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

#[derive(Clone)]
pub struct L2SyncService {
    db_backend: Arc<MadaraBackend>,
    block_importer: Arc<BlockImporter>,
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    starting_block: Option<u64>,
    start_params: Option<TelemetryHandle>,
    disabled: bool,
    pending_block_poll_interval: Duration,
}

impl L2SyncService {
    pub async fn new(
        config: &SyncParams,
        chain_config: Arc<ChainConfig>,
        db: &DatabaseService,
        block_importer: Arc<BlockImporter>,
        telemetry: TelemetryHandle,
        warp_update: bool,
    ) -> anyhow::Result<Self> {
        let fetch_config = config.block_fetch_config(chain_config.chain_id.clone(), chain_config.clone(), warp_update);

        tracing::info!("🛰️  Using feeder gateway URL: {}", fetch_config.feeder_gateway.as_str());

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            fetch_config,
            starting_block: config.unsafe_starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_importer,
            start_params: Some(telemetry),
            disabled: config.sync_disabled,
            pending_block_poll_interval: config.pending_block_poll_interval,
        })
    }
}

#[async_trait::async_trait]
impl Service for L2SyncService {
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>, ctx: ServiceContext) -> anyhow::Result<()> {
        if self.disabled {
            return Ok(());
        }
        let L2SyncService {
            fetch_config,
            backup_every_n_blocks,
            starting_block,
            pending_block_poll_interval,
            block_importer,
            ..
        } = self.clone();
        let telemetry = self.start_params.take().context("Service already started")?;

        let db_backend = Arc::clone(&self.db_backend);

        join_set.spawn(async move {
            mc_sync::l2_sync_worker(
                &db_backend,
                ctx,
                fetch_config,
                SyncConfig {
                    block_importer,
                    starting_block,
                    backup_every_n_blocks,
                    telemetry,
                    pending_block_poll_interval,
                },
            )
            .await
        });

        Ok(())
    }

    fn id(&self) -> MadaraService {
        MadaraService::L2Sync
    }
}
