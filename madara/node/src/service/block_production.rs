use crate::cli::block_production::BlockProductionParams;
use anyhow::Context;
use mc_block_production::{metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask};
use mc_db::MadaraBackend;
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_settlement_client::SettlementClient;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use std::{io::Write, sync::Arc};

pub struct BlockProductionService {
    backend: Arc<MadaraBackend>,
    mempool: Arc<mc_mempool::Mempool>,
    metrics: Arc<BlockProductionMetrics>,
    l1_client: Arc<dyn SettlementClient>,
    no_charge_fee: bool,
    n_devnet_contracts: u64,
    disabled: bool,
    /// The current task handle, if the service is running.
    /// This is recreated on each start to support service restarts.
    current_handle: Option<BlockProductionHandle>,
}

impl BlockProductionService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: &BlockProductionParams,
        backend: &Arc<MadaraBackend>,
        mempool: Arc<mc_mempool::Mempool>,
        l1_client: Arc<dyn SettlementClient>,
        no_charge_fee: bool,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(BlockProductionMetrics::register());

        Ok(Self {
            backend: backend.clone(),
            mempool,
            metrics,
            l1_client,
            no_charge_fee,
            n_devnet_contracts: config.devnet_contracts,
            disabled: config.block_production_disabled,
            current_handle: None,
        })
    }

    /// Creates a new BlockProductionTask for this service.
    /// Called on each start to support service restarts.
    fn create_task(&self) -> BlockProductionTask {
        BlockProductionTask::new(
            self.backend.clone(),
            self.mempool.clone(),
            self.metrics.clone(),
            self.l1_client.clone(),
            self.no_charge_fee,
        )
    }
}

#[async_trait::async_trait]
impl Service for BlockProductionService {
    #[tracing::instrument(skip(self, runner), fields(module = "BlockProductionService"))]
    async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        if !self.disabled {
            // Create a fresh task on each start to support service restarts
            let block_production_task = self.create_task();
            // Store the handle for external access
            self.current_handle = Some(block_production_task.handle());
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

        let keys = if backend.latest_confirmed_block_n().is_none() {
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
        self.current_handle.clone().expect("Service not started")
    }
}
