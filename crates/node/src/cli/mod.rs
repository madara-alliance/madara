pub mod block_production;
pub mod chain_config_overrides;
pub mod db;
pub mod l1;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

use crate::cli::l1::L1SyncParams;
pub use block_production::*;
pub use chain_config_overrides::*;
pub use db::*;
pub use prometheus::*;
pub use rpc::*;
use starknet_api::core::ChainId;
pub use sync::*;
pub use telemetry::*;

use clap::ArgGroup;
use mp_chain_config::ChainConfig;
use std::path::PathBuf;
use std::sync::Arc;
use url::Url;

/// Madara: High performance Starknet sequencer/full-node.
#[derive(Clone, Debug, clap::Parser)]
#[clap(
    group(
        ArgGroup::new("mode")
            .args(&["sequencer", "full", "devnet"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("chain_config")
            .args(&["chain_config_path", "preset"])
            .requires("sequencer")
            .requires("devnet")
    ),
    group(
        ArgGroup::new("full_mode_config")
            .args(&["network", "chain_config_path", "preset"])
            .requires("full")
    ),
)]

pub struct RunCmd {
    /// The human-readable name for this node.
    /// It is used as the network node name.
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub db_params: DbParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub sync_params: SyncParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub l1_sync_params: L1SyncParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub telemetry_params: TelemetryParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub prometheus_params: PrometheusParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub rpc_params: RpcParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub block_production_params: BlockProductionParams,

    /// The node will run as a sequencer and produce its own state.
    #[arg(long, group = "mode")]
    pub sequencer: bool,

    /// The node will run as a full node and sync the state of a specific network from others.
    #[arg(long, group = "mode")]
    pub full: bool,

    /// The node will run as a testing sequencer with predeployed contracts.
    #[arg(long, group = "mode")]
    pub devnet: bool,

    /// The network chain configuration.
    #[clap(long, short, group = "full_mode_config")]
    pub network: Option<NetworkType>,

    /// Chain configuration file path.
    #[clap(long, value_name = "CHAIN CONFIG FILE PATH", group = "chain_config")]
    pub chain_config_path: Option<PathBuf>,

    /// Use preset as chain Config
    #[clap(long, value_name = "PRESET NAME", group = "chain_config")]
    pub preset: Option<String>,

    /// Overrides parameters from the Chain Config.
    #[clap(flatten)]
    pub chain_config_override: ChainConfigOverrideParams,
}

impl RunCmd {
    pub async fn node_name_or_provide(&mut self) -> &str {
        if self.name.is_none() {
            let name = crate::util::get_random_pokemon_name().await.unwrap_or_else(|e| {
                log::warn!("Failed to get random pokemon name: {}", e);
                "madara".to_string()
            });

            self.name = Some(name);
        }
        self.name.as_ref().expect("Name was just set")
    }

    pub fn chain_config(&self) -> anyhow::Result<Arc<ChainConfig>> {
        let mut chain_config = match &self.preset {
            Some(preset_name) => ChainConfig::from_preset_name(preset_name.as_str()).map_err(|err| {
                log::error!("Failed to load config from preset '{}': {}", preset_name, err);
                anyhow::anyhow!(err)
            })?,
            None => {
                let path = self.chain_config_path.clone().ok_or_else(|| {
                    log::error!("{}", "Chain config path is not set");
                    anyhow::anyhow!(if self.is_sequencer() {
                        "In Sequencer or Devnet mode, you must define a Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`.\nThe default presets are:\n- 'mainnet' - (configs/presets/mainnet.yml)\n- 'sepolia' - (configs/presets/sepolia.yml)\n- 'integration' - (configs/presets/integration.yml)"
                    } else {
                        "No network specified. Please provide a network with `--network <NETWORK>` or a custom Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`.\nThe default presets are:\n- 'mainnet' - (configs/presets/mainnet.yml)\n- 'sepolia' - (configs/presets/sepolia.yml)\n- 'integration' - (configs/presets/integration.yml)"
                    })
                })?;
                ChainConfig::from_yaml(&path).map_err(|err| {
                    log::error!("Failed to load config from YAML at path '{}': {}", path.display(), err);
                    anyhow::anyhow!(err)
                })?
            }
        };

        if !self.chain_config_override.overrides.is_empty() {
            chain_config = self.chain_config_override.override_chain_config(chain_config)?;
        }

        Ok(Arc::new(chain_config))
    }

    /// Assigns a specific ChainConfig based on a defined network.
    pub fn set_preset_from_network(&self) -> anyhow::Result<Arc<ChainConfig>> {
        let mut chain_config = match self.network {
            Some(NetworkType::Main) => ChainConfig::starknet_mainnet(),
            Some(NetworkType::Test) => ChainConfig::starknet_sepolia(),
            Some(NetworkType::Integration) => ChainConfig::starknet_integration(),
            Some(NetworkType::Devnet) => ChainConfig::madara_devnet(),
            None => {
                log::error!("{}", "Chain config path is not set");
                anyhow::bail!("No network specified. Please provide a network with `--network <NETWORK>` or a custom Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`")
            }
        };

        if !self.chain_config_override.overrides.is_empty() {
            chain_config = self.chain_config_override.override_chain_config(chain_config)?;
        }

        Ok(Arc::new(chain_config))
    }

    pub fn is_sequencer(&self) -> bool {
        self.sequencer || self.devnet
    }
}

/// Starknet network types.
#[derive(Debug, Clone, Copy, clap::ValueEnum, PartialEq)]
pub enum NetworkType {
    /// The main network (mainnet). Alias: mainnet
    #[value(alias("mainnet"))]
    Main,
    /// The test network (testnet). Alias: sepolia
    #[value(alias("sepolia"))]
    Test,
    /// The integration network.
    Integration,
    /// A devnet for local testing
    #[value(alias("devnet"))]
    Devnet,
}

impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha-sepolia.starknet.io",
            NetworkType::Integration => "https://integration-sepolia.starknet.io",
            NetworkType::Devnet => unreachable!("Gateway url isn't needed for a devnet sequencer"),
        }
    }

    pub fn gateway(&self) -> Url {
        format!("{}/gateway", self.uri()).parse().expect("Invalid uri")
    }

    pub fn feeder_gateway(&self) -> Url {
        format!("{}/feeder_gateway", self.uri()).parse().expect("Invalid uri")
    }

    pub fn chain_id(&self) -> ChainId {
        match self {
            NetworkType::Main => ChainId::Mainnet,
            NetworkType::Test => ChainId::Sepolia,
            NetworkType::Integration => ChainId::IntegrationSepolia,
            NetworkType::Devnet => ChainId::Other("MADARA_TEST".to_string()),
        }
    }
}
