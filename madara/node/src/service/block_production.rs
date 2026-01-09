use crate::cli::block_production::BlockProductionParams;
use anyhow::Context;
use mc_block_production::{metrics::BlockProductionMetrics, BlockProductionTask, SharedBlockProductionHandle};
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
    /// The initial task created at construction time.
    /// Used to get the handle before service starts, then consumed on first start.
    initial_task: Option<BlockProductionTask>,
    /// Shared handle that can be updated on service restart.
    /// RPC holds a clone of this Arc and always reads the current handle.
    shared_handle: SharedBlockProductionHandle,
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

        // Create initial task so handle() can be called before start()
        let initial_task = BlockProductionTask::new(
            backend.clone(),
            mempool.clone(),
            metrics.clone(),
            l1_client.clone(),
            no_charge_fee,
        );

        // Create shared handle with the initial task's handle
        let shared_handle = Arc::new(tokio::sync::RwLock::new(Some(initial_task.handle())));

        Ok(Self {
            backend: backend.clone(),
            mempool,
            metrics,
            l1_client,
            no_charge_fee,
            n_devnet_contracts: config.devnet_contracts,
            disabled: config.block_production_disabled,
            initial_task: Some(initial_task),
            shared_handle,
        })
    }

    /// Creates a new BlockProductionTask for this service.
    /// Called on restarts after the initial task has been consumed.
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
            // Use initial task on first start, create fresh task on restarts
            let block_production_task = match self.initial_task.take() {
                Some(task) => task,
                None => {
                    // Restart case: create new task and update shared handle
                    let task = self.create_task();
                    let new_handle = task.handle();
                    *self.shared_handle.write().await = Some(new_handle);
                    tracing::info!("🔄 Block production service restarted with new handle");
                    task
                }
            };
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

    /// Returns a shared handle that is automatically updated when the service restarts.
    /// RPC should use this instead of handle() to always have access to the current handle.
    pub fn shared_handle(&self) -> SharedBlockProductionHandle {
        self.shared_handle.clone()
    }
}
