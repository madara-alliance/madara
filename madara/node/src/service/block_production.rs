use crate::cli::block_production::{parse_hex_felt, BlockProductionParams, StartupExecutionModeParam};
use anyhow::{bail, Context};
use figment::{
    providers::{Format, Json, Toml, Yaml},
    Figment,
};
use mc_block_production::fallback::types::StartupExecutionMode;
use mc_block_production::{metrics::BlockProductionMetrics, BlockProductionHandle, BlockProductionTask};
use mc_db::MadaraBackend;
use mc_devnet::{ChainGenesisDescription, DevnetKeys};
use mc_settlement_client::SettlementClient;
use mp_utils::service::{MadaraServiceId, PowerOfTwo, Service, ServiceId, ServiceRunner};
use serde::Deserialize;
use starknet_types_core::felt::Felt;
use std::{io::Write, path::Path, sync::Arc};

fn parse_felt_list(values: &[String], field_name: &str) -> anyhow::Result<Vec<Felt>> {
    values
        .iter()
        .map(|value| parse_hex_felt(value).map_err(anyhow::Error::msg))
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to parse {field_name}"))
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct RustExecRoutingConfigFile {
    executor_addresses: Vec<String>,
    supported_selectors: Vec<String>,
    supported_class_hashes: Vec<String>,
    rust_batch_size: u64,
    blockifier_batch_size: u64,
}

fn load_rust_exec_routing_config(path: &Path) -> anyhow::Result<RustExecRoutingConfigFile> {
    let config = match path.extension().and_then(|ext| ext.to_str()) {
        Some("yaml") | Some("yml") => Figment::new().merge(Yaml::file(path)),
        Some("json") => Figment::new().merge(Json::file(path)),
        Some("toml") => Figment::new().merge(Toml::file(path)),
        _ => bail!(
            "unsupported RustExec routing config file type for '{}'; use .yaml, .yml, .json, or .toml",
            path.display()
        ),
    };

    config.extract().with_context(|| format!("failed to load RustExec routing config from {}", path.display()))
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
        let mut task =
            BlockProductionTask::new(backend.clone(), mempool, metrics, l1_client, mempool_paused, no_charge_fee)
                .with_replay_mode_enabled(config.replay_mode)
                .with_startup_execution_mode(match config.startup_execution_mode {
                    StartupExecutionModeParam::Mixed => StartupExecutionMode::Mixed,
                    StartupExecutionModeParam::BlockifierOnly => StartupExecutionMode::BlockifierOnly,
                });

        if let Some(routing_config_path) = &config.rust_exec_routing_config {
            let routing_cfg = load_rust_exec_routing_config(routing_config_path)?;
            let rust_batch_size =
                usize::try_from(routing_cfg.rust_batch_size).context("rust_batch_size does not fit into usize")?;
            let blockifier_batch_size = usize::try_from(routing_cfg.blockifier_batch_size)
                .context("blockifier_batch_size does not fit into usize")?;

            task = task
                .with_rust_exec_executor_addresses(parse_felt_list(
                    &routing_cfg.executor_addresses,
                    "executor_addresses",
                )?)
                .with_rust_exec_supported_selectors(parse_felt_list(
                    &routing_cfg.supported_selectors,
                    "supported_selectors",
                )?)
                .with_rust_exec_supported_class_hashes(parse_felt_list(
                    &routing_cfg.supported_class_hashes,
                    "supported_class_hashes",
                )?)
                .with_rust_exec_batch_size(rust_batch_size)
                .with_rust_exec_blockifier_batch_size(blockifier_batch_size);
        }

        if let Some(executor_addresses) = &config.rust_exec_executor_addresses {
            task = task.with_rust_exec_executor_addresses(parse_felt_list(
                executor_addresses,
                "rust_exec_executor_addresses",
            )?);
        }

        if let Some(supported_selectors) = &config.rust_exec_supported_selectors {
            task = task.with_rust_exec_supported_selectors(parse_felt_list(
                supported_selectors,
                "rust_exec_supported_selectors",
            )?);
        }

        if let Some(supported_class_hashes) = &config.rust_exec_supported_class_hashes {
            task = task.with_rust_exec_supported_class_hashes(parse_felt_list(
                supported_class_hashes,
                "rust_exec_supported_class_hashes",
            )?);
        }

        if let Some(rust_batch_size) = config.rust_exec_batch_size_override {
            let rust_batch_size =
                usize::try_from(rust_batch_size).context("rust_exec_batch_size_override does not fit into usize")?;
            task = task.with_rust_exec_batch_size(rust_batch_size);
        }

        if let Some(blockifier_batch_size) = config.rust_exec_blockifier_batch_size_override {
            let blockifier_batch_size = usize::try_from(blockifier_batch_size)
                .context("rust_exec_blockifier_batch_size_override does not fit into usize")?;
            task = task.with_rust_exec_blockifier_batch_size(blockifier_batch_size);
        }

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
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{load_rust_exec_routing_config, RustExecRoutingConfigFile};

    fn temp_file_path(extension: &str) -> PathBuf {
        let unique =
            SystemTime::now().duration_since(UNIX_EPOCH).expect("system time should be after unix epoch").as_nanos();
        std::env::temp_dir().join(format!(
            "madara_rust_exec_routing_config_{}_{}.{}",
            std::process::id(),
            unique,
            extension
        ))
    }

    #[test]
    fn load_rust_exec_routing_config_yaml_file() {
        let path = temp_file_path("yaml");
        fs::write(
            &path,
            r#"executor_addresses:
  - "0x1"
supported_selectors:
  - "0x2"
supported_class_hashes:
  - "0x3"
rust_batch_size: 30
blockifier_batch_size: 10
"#,
        )
        .expect("should write temp config");

        let config = load_rust_exec_routing_config(&path).expect("routing config should load");
        assert_eq!(
            config,
            RustExecRoutingConfigFile {
                executor_addresses: vec!["0x1".to_string()],
                supported_selectors: vec!["0x2".to_string()],
                supported_class_hashes: vec!["0x3".to_string()],
                rust_batch_size: 30,
                blockifier_batch_size: 10,
            }
        );

        fs::remove_file(path).expect("should remove temp config");
    }

    #[test]
    fn load_rust_exec_routing_config_rejects_unsupported_extension() {
        let path = temp_file_path("txt");
        fs::write(&path, "ignored").expect("should write temp config");

        let err = load_rust_exec_routing_config(&path).expect_err("unsupported extension must fail");
        assert!(err.to_string().contains("unsupported RustExec routing config file type"));

        fs::remove_file(path).expect("should remove temp config");
    }
}
