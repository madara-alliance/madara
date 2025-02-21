use crate::cli::L2SyncParams;
use mc_block_import::BlockImporter;
use mc_db::{DatabaseService, MadaraBackend};
use mc_sync::fetch::fetchers::{FetchConfig, WarpUpdateConfig};
use mc_sync::SyncConfig;
use mc_telemetry::TelemetryHandle;
use mp_chain_config::ChainConfig;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct L2SyncService {
    db_backend: Arc<MadaraBackend>,
    block_importer: Arc<BlockImporter>,
    fetch_config: FetchConfig,
    backup_every_n_blocks: Option<u64>,
    starting_block: Option<u64>,
    telemetry: Arc<TelemetryHandle>,
    pending_block_poll_interval: Duration,
}

impl L2SyncService {
    pub async fn new(
        config: &L2SyncParams,
        chain_config: Arc<ChainConfig>,
        db: &DatabaseService,
        block_importer: Arc<BlockImporter>,
        telemetry: TelemetryHandle,
        warp_update: Option<WarpUpdateConfig>,
    ) -> anyhow::Result<Self> {
        let fetch_config = config.block_fetch_config(chain_config.chain_id.clone(), chain_config.clone(), warp_update);

        tracing::info!("🛰️ Using feeder gateway URL: {}", fetch_config.feeder_gateway.as_str());

        Ok(Self {
            db_backend: Arc::clone(db.backend()),
            fetch_config,
            starting_block: config.unsafe_starting_block,
            backup_every_n_blocks: config.backup_every_n_blocks,
            block_importer,
            telemetry: Arc::new(telemetry),
            pending_block_poll_interval: config.pending_block_poll_interval,
        })
    }
}

#[async_trait::async_trait]
impl Service for L2SyncService {
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let L2SyncService {
            db_backend,
            fetch_config,
            backup_every_n_blocks,
            starting_block,
            pending_block_poll_interval,
            block_importer,
            telemetry,
        } = self.clone();
        let telemetry = Arc::clone(&telemetry);

        runner.service_loop(move |ctx| {
            mc_sync::l2_sync_worker(
                db_backend,
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
        });

        Ok(())
    }
}

impl ServiceId for L2SyncService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::L2Sync.svc_id()
    }
}
