use std::{io::Write, sync::Arc};

use anyhow::Context;
use mc_block_import::{BlockImporter, BlockValidationContext};
use mc_db::{DatabaseService, MadaraBackend};
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_mempool::{block_production::BlockProductionTask, L1DataProvider, Mempool};
use mc_metrics::MetricsRegistry;
use mc_telemetry::TelemetryHandle;
use mp_utils::service::Service;
use tokio::task::JoinSet;

use crate::cli::block_production::BlockProductionParams;

struct StartParams {
    backend: Arc<MadaraBackend>,
    block_import: Arc<BlockImporter>,
    mempool: Arc<Mempool>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    is_devnet: bool,
    n_devnet_contracts: u64,
}

pub struct BlockProductionService {
    start: Option<StartParams>,
    enabled: bool,
}
impl BlockProductionService {
    pub fn new(
        config: &BlockProductionParams,
        db_service: &DatabaseService,
        mempool: Arc<mc_mempool::Mempool>,
        block_import: Arc<BlockImporter>,
        l1_data_provider: Arc<dyn L1DataProvider>,
        _metrics_handle: &MetricsRegistry,
        _telemetry: TelemetryHandle,
    ) -> anyhow::Result<Self> {
        if config.block_production_disabled {
            return Ok(Self { start: None, enabled: false });
        }

        Ok(Self {
            start: Some(StartParams {
                backend: Arc::clone(db_service.backend()),
                l1_data_provider,
                mempool,
                block_import,
                n_devnet_contracts: config.devnet_contracts,
                is_devnet: config.devnet,
            }),
            enabled: true,
        })
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    // TODO(cchudant,2024-07-30): special threading requirements for the block production task
    async fn start(&mut self, join_set: &mut JoinSet<anyhow::Result<()>>) -> anyhow::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let StartParams { backend, l1_data_provider, mempool, is_devnet, n_devnet_contracts, block_import } =
            self.start.take().expect("Service already started");

        if is_devnet {
            // DEVNET: we the genesis block for the devnet if not deployed, otherwise we only print the devnet keys.

            let keys = if backend.get_latest_block_n().context("Getting the latest block number in db")?.is_none() {
                // deploy devnet genesis

                log::info!("⛏️  Deploying devnet genesis block");

                let mut genesis_config =
                    ChainGenesisDescription::base_config().context("Failed to create base genesis config")?;
                let contracts = genesis_config
                    .add_devnet_contracts(n_devnet_contracts)
                    .context("Failed to add devnet contracts")?;

                let genesis_block = genesis_config
                    .build(backend.chain_config())
                    .context("Building genesis block from devnet config")?;

                block_import
                    .add_block(
                        genesis_block,
                        BlockValidationContext::new(backend.chain_config().chain_id.clone()).trust_class_hashes(true),
                    )
                    .await
                    .context("Importing devnet genesis block")?;

                contracts.save_to_db(&backend).context("Saving predeployed devnet contract keys to database")?;

                contracts
            } else {
                DevnetKeys::from_db(&backend).context("Getting the devnet predeployed contract keys and balances")?
            };

            // display devnet welcome message :)
            // we display it to stdout instead of stderr

            let msg = format!("{}", keys);

            std::io::stdout().write(msg.as_bytes()).context("Writing devnet welcome message to stdout")?;
        }

        join_set.spawn(async move {
            BlockProductionTask::new(backend, block_import, mempool, l1_data_provider)?.block_production_task().await?;
            Ok(())
        });

        Ok(())
    }
}
