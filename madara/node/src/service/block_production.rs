use crate::cli::block_production::{BlockProductionParams, ParallelMerkleTrieLogMode as CliParallelMerkleTrieLogMode};
use anyhow::Context;
use mc_block_production::{
    metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask, ParallelMerkleConfig,
    ParallelMerkleTrieLogMode,
};
use mc_db::MadaraBackend;
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
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
    fn parallel_merkle_config(config: &BlockProductionParams) -> ParallelMerkleConfig {
        ParallelMerkleConfig {
            enabled: config.parallel_merkle_enabled,
            flush_interval: config.parallel_merkle_flush_interval,
            max_inflight: config.parallel_merkle_max_inflight,
            trie_log_mode: match config.parallel_merkle_trie_log_mode {
                CliParallelMerkleTrieLogMode::Off => ParallelMerkleTrieLogMode::Off,
                CliParallelMerkleTrieLogMode::Checkpoint => ParallelMerkleTrieLogMode::Checkpoint,
            },
        }
    }

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
            task: Some(BlockProductionTask::new(
                backend.clone(),
                mempool,
                metrics,
                l1_client,
                no_charge_fee,
                Self::parallel_merkle_config(config),
            )?),
            n_devnet_contracts: config.devnet_contracts,
            disabled: config.block_production_disabled,
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
        self.task.as_ref().expect("Service started").handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::block_production::ParallelMerkleTrieLogMode as CliMode;
    use mc_db::MadaraBackend;
    use mc_mempool::{Mempool, MempoolConfig};
    use mp_chain_config::ChainConfig;
    use std::sync::Arc;

    #[test]
    fn parallel_merkle_config_maps_defaults() {
        let config = BlockProductionParams {
            block_production_disabled: false,
            devnet_contracts: 10,
            parallel_merkle_enabled: false,
            parallel_merkle_flush_interval: 3,
            parallel_merkle_max_inflight: 10,
            parallel_merkle_trie_log_mode: CliMode::Off,
        };

        let cfg = BlockProductionService::parallel_merkle_config(&config);
        assert!(!cfg.enabled);
        assert_eq!(cfg.flush_interval, 3);
        assert_eq!(cfg.max_inflight, 10);
        assert_eq!(cfg.trie_log_mode, ParallelMerkleTrieLogMode::Off);
    }

    #[test]
    fn service_new_propagates_parallel_merkle_config_to_task() {
        let config = BlockProductionParams {
            block_production_disabled: false,
            devnet_contracts: 10,
            parallel_merkle_enabled: true,
            parallel_merkle_flush_interval: 9,
            parallel_merkle_max_inflight: 77,
            parallel_merkle_trie_log_mode: CliMode::Checkpoint,
        };

        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let mempool = Arc::new(Mempool::new(backend.clone(), MempoolConfig::default()));
        let l1_client: Arc<dyn SettlementClient> = Arc::new(mc_settlement_client::L1SyncDisabledClient);

        let service = BlockProductionService::new(&config, &backend, mempool, l1_client, /* no_charge_fee */ true)
            .expect("service should be created");

        let task_cfg =
            service.task.as_ref().expect("task should be present before start").parallel_merkle_config_for_test();

        assert!(task_cfg.enabled);
        assert_eq!(task_cfg.flush_interval, 9);
        assert_eq!(task_cfg.max_inflight, 77);
        assert_eq!(task_cfg.trie_log_mode, ParallelMerkleTrieLogMode::Checkpoint);
    }
}
