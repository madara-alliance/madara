use crate::cli::block_production::BlockProductionParams;
use anyhow::Context;
use mc_block_production::{metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask};
use mc_db::{DatabaseService, MadaraBackend};
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_mempool::L1DataProvider;
use mc_settlement_client::SettlementClient;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::{io::Write, sync::Arc};

pub struct BlockProductionService {
    backend: Arc<MadaraBackend>,
    task: Option<BlockProductionTask>,
    n_devnet_contracts: u64,
    disabled: bool,
}

impl BlockProductionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &BlockProductionParams,
        db_service: &DatabaseService,
        mempool: Arc<mc_mempool::Mempool>,
        l1_data_provider: Arc<dyn L1DataProvider>,
        l1_client: Arc<dyn SettlementClient>,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(BlockProductionMetrics::register());

        Ok(Self {
            task: Some(BlockProductionTask::new(
                db_service.backend().clone(),
                mempool,
                metrics,
                l1_data_provider,
                l1_client,
            )),
            n_devnet_contracts: config.devnet_contracts,
            disabled: config.block_production_disabled,
            backend: db_service.backend().clone(),
        })
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    #[tracing::instrument(skip(self, runner), fields(module = "BlockProductionService"))]
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        let block_production_task = self.task.take().context("Service already started")?;
        if !self.disabled {
            runner.service_loop(move |ctx| block_production_task.run(ctx));
        }

        Ok(())
    }
}

impl ServiceId for BlockProductionService {
    #[inline(always)]
    fn svc_id(&self) -> PowerOfTwo {
        MadaraServiceId::BlockProduction.svc_id()
    }
}

impl BlockProductionService {
    /// Initializes the genesis state of a devnet. This is needed for local sequencers.
    ///
    /// This methods was made external to [Service::start] as it needs to be
    /// called on node startup even if sequencer block production is not yet
    /// enabled. This happens during warp updates on a local sequencer.
    pub async fn setup_devnet(&self) -> anyhow::Result<()> {
        let Self { backend, n_devnet_contracts, .. } = self;

        let keys = if backend.get_latest_block_n().context("Getting the latest block number in db")?.is_none() {
            // deploy devnet genesis
            tracing::info!("⛏️  Deploying devnet genesis block");

            let mut genesis_config =
                ChainGenesisDescription::base_config().context("Failed to create base genesis config")?;
            let contracts =
                genesis_config.add_devnet_contracts(*n_devnet_contracts).context("Failed to add devnet contracts")?;

            contracts.save_to_db(backend)?;

            // Deploy genesis block
            genesis_config.build_and_store(backend).await.context("Building and storing genesis block")?;

            contracts
        } else {
            DevnetKeys::from_db(backend).context("Getting the devnet predeployed contract keys and balances")?
        };

        // display devnet welcome message :)
        // we display it to stdout instead of stderr
        let msg = format!("{}", keys);
        std::io::stdout().write(msg.as_bytes()).context("Writing devnet welcome message to stdout")?;

        anyhow::Ok(())
    }

    pub fn handle(&self) -> BlockProductionHandle {
        self.task.as_ref().expect("Service started").handle()
    }
}
