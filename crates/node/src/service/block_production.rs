use std::{io::Write, sync::Arc};

use anyhow::Context;
use mc_block_import::{BlockImporter, BlockValidationContext};
use mc_db::{DatabaseService, MadaraBackend};
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_mempool::{
    block_production::BlockProductionTask, block_production_metrics::BlockProductionMetrics, L1DataProvider, Mempool,
};
use mp_utils::service::{MadaraService, Service, ServiceRunner};

use crate::cli::block_production::BlockProductionParams;

pub struct BlockProductionService {
    backend: Arc<MadaraBackend>,
    block_import: Arc<BlockImporter>,
    mempool: Arc<Mempool>,
    metrics: Arc<BlockProductionMetrics>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    n_devnet_contracts: u64,
}

impl BlockProductionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &BlockProductionParams,
        db_service: &DatabaseService,
        mempool: Arc<mc_mempool::Mempool>,
        block_import: Arc<BlockImporter>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(BlockProductionMetrics::register());

        Ok(Self {
            backend: Arc::clone(db_service.backend()),
            l1_data_provider,
            mempool,
            metrics,
            block_import,
            n_devnet_contracts: config.devnet_contracts,
        })
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    // TODO(cchudant,2024-07-30): special threading requirements for the block production task
    #[tracing::instrument(skip(self, runner), fields(module = "BlockProductionService"))]
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let Self { backend, l1_data_provider, mempool, metrics, n_devnet_contracts, block_import } = self;

        // DEVNET: we the genesis block for the devnet if not deployed, otherwise we only print the devnet keys.

        let keys = if backend.get_latest_block_n().context("Getting the latest block number in db")?.is_none() {
            // deploy devnet genesis
            tracing::info!("⛏️  Deploying devnet genesis block");

            let mut genesis_config =
                ChainGenesisDescription::base_config().context("Failed to create base genesis config")?;
            let contracts =
                genesis_config.add_devnet_contracts(*n_devnet_contracts).context("Failed to add devnet contracts")?;

            let genesis_block =
                genesis_config.build(backend.chain_config()).context("Building genesis block from devnet config")?;

            block_import
                .add_block(
                    genesis_block,
                    BlockValidationContext::new(backend.chain_config().chain_id.clone()).trust_class_hashes(true),
                )
                .await
                .context("Importing devnet genesis block")?;

            contracts.save_to_db(backend).context("Saving predeployed devnet contract keys to database")?;

            contracts
        } else {
            DevnetKeys::from_db(backend).context("Getting the devnet predeployed contract keys and balances")?
        };

        // display devnet welcome message :)
        // we display it to stdout instead of stderr
        let msg = format!("{}", keys);
        std::io::stdout().write(msg.as_bytes()).context("Writing devnet welcome message to stdout")?;

        let block_production_task = BlockProductionTask::new(
            Arc::clone(backend),
            Arc::clone(block_import),
            Arc::clone(mempool),
            Arc::clone(metrics),
            Arc::clone(l1_data_provider),
        )?;

        runner.service_loop(move |ctx| block_production_task.block_production_task(ctx));

        Ok(())
    }

    fn id(&self) -> MadaraService {
        MadaraService::BlockProduction
    }
}
