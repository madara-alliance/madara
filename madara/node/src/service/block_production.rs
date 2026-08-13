use crate::cli::block_production::{
    parse_hex_felt, BlockProductionParams, RustExecCanonicalSourceParam, RustExecParams, StartupExecutionModeParam,
};
use anyhow::Context;
use mc_block_production::fallback::types::StartupExecutionMode;
use mc_block_production::{
    metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask, RustExecCanonicalSource,
    RustExecRuntimeOptions,
};
use mc_db::MadaraBackend;
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_settlement_client::SettlementClient;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use starknet_types_core::felt::Felt;
use std::{io::Write, sync::Arc};

fn parse_felt_list(values: &[String], field_name: &str) -> anyhow::Result<Vec<Felt>> {
    values
        .iter()
        .map(|value| parse_hex_felt(value).map_err(anyhow::Error::msg))
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to parse {field_name}"))
}

fn rust_exec_runtime_options(config: &RustExecParams) -> RustExecRuntimeOptions {
    RustExecRuntimeOptions {
        conversion_log: config.conversion_log,
        execution_log: config.execution_log,
        execution_log_inner: config.execution_log_inner,
        tx_diff_log: config.tx_diff_log,
        debug_block: config.debug_block,
        inner_timing_log: config.inner_timing_log,
        ctx_cache: config.ctx_cache,
        pedersen_cache: config.pedersen_cache,
        precomputed_sn_keccak: config.precomputed_sn_keccak,
        hash_agg_logs: config.hash_agg_logs,
        storage_agg_logs: config.storage_agg_logs,
        ignore_fee_mismatch: config.ignore_fee_mismatch,
        ignore_fee_token_mismatch: config.ignore_fee_token_mismatch,
        ignored_storage_mismatch_canonical_source: match config.ignored_storage_mismatch_canonical_source {
            RustExecCanonicalSourceParam::ExecutionBox => RustExecCanonicalSource::ExecutionBox,
            RustExecCanonicalSourceParam::BlockifierReexec => RustExecCanonicalSource::BlockifierReexec,
        },
        settle_trade_v3_positions: config.settle_trade_v3_positions,
    }
}

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
        backend: &Arc<MadaraBackend>,
        mempool: Arc<mc_mempool::Mempool>,
        l1_client: Arc<dyn SettlementClient>,
        no_charge_fee: bool,
    ) -> anyhow::Result<Self> {
        let metrics = Arc::new(BlockProductionMetrics::register());
        let mempool_paused = config.mempool_paused;
        let close_queue_capacity =
            usize::try_from(config.close_queue_capacity).context("close_queue_capacity does not fit into usize")?;
        let mut task =
            BlockProductionTask::new(backend.clone(), mempool, metrics, l1_client, mempool_paused, no_charge_fee)
                .with_replay_mode_enabled(config.replay_mode)
                .with_startup_execution_mode(match config.startup_execution_mode {
                    StartupExecutionModeParam::Mixed => StartupExecutionMode::Mixed,
                    StartupExecutionModeParam::BlockifierOnly => StartupExecutionMode::BlockifierOnly,
                })
                .with_close_queue_capacity(close_queue_capacity)?;

        let rust_batch_size =
            usize::try_from(config.rust_exec.batch_size).context("rust_exec.batch_size does not fit into usize")?;
        let blockifier_batch_size = usize::try_from(config.rust_exec.blockifier_batch_size)
            .context("rust_exec.blockifier_batch_size does not fit into usize")?;

        task = task
            .with_rust_exec_executor_addresses(parse_felt_list(
                &config.rust_exec.executor_addresses,
                "rust_exec.executor_addresses",
            )?)
            .with_rust_exec_batch_size(rust_batch_size)
            .with_rust_exec_blockifier_batch_size(blockifier_batch_size)
            .with_rust_exec_runtime_options(rust_exec_runtime_options(&config.rust_exec));

        Ok(Self {
            backend: backend.clone(),
            task: Some(task),
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
    use super::{rust_exec_runtime_options, RustExecParams};

    #[test]
    fn rust_exec_runtime_options_carry_cli_values() {
        let config = RustExecParams {
            execution_log: true,
            ctx_cache: false,
            precomputed_sn_keccak: true,
            settle_trade_v3_positions: Some(75),
            ..Default::default()
        };

        let options = rust_exec_runtime_options(&config);

        assert!(options.execution_log);
        assert!(!options.ctx_cache);
        assert!(options.precomputed_sn_keccak);
        assert_eq!(options.settle_trade_v3_positions, Some(75));
    }
}
