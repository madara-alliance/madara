pub mod analytics;
pub mod block_production;
pub mod chain_config_overrides;
pub mod db;
pub mod gateway;
pub mod l1;
pub mod rpc;
pub mod sync;
pub mod telemetry;
use crate::cli::l1::L1SyncParams;
use analytics::AnalyticsParams;
pub use block_production::*;
pub use chain_config_overrides::*;
pub use db::*;
pub use gateway::*;
pub use rpc::*;
use starknet_api::core::ChainId;
use std::str::FromStr;
pub use sync::*;
pub use telemetry::*;

use clap::ArgGroup;
use mp_chain_config::ChainConfig;
use std::path::PathBuf;
use std::sync::Arc;

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
    #[arg(env = "MADARA_NAME", long, value_name = "NAME")]
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
    pub analytics_params: AnalyticsParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub telemetry_params: TelemetryParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub gateway_params: GatewayParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub rpc_params: RpcParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub block_production_params: BlockProductionParams,

    /// The node will run as a sequencer and produce its own state.
    #[arg(env = "MADARA_SEQUENCER", long, group = "mode")]
    pub sequencer: bool,

    /// The node will run as a full node and sync the state of a specific network from others.
    #[arg(env = "MADARA_FULL", long, group = "mode")]
    pub full: bool,

    /// The node will run as a testing sequencer with predeployed contracts.
    #[arg(env = "MADARA_DEVNET", long, group = "mode")]
    pub devnet: bool,

    /// The network chain configuration.
    #[clap(env = "MADARA_NETWORK", long, short, group = "full_mode_config")]
    pub network: Option<NetworkType>,

    /// Chain configuration file path.
    #[clap(env = "MADARA_CHAIN_CONFIG_PATH", long, value_name = "CHAIN CONFIG FILE PATH", group = "chain_config")]
    pub chain_config_path: Option<PathBuf>,

    /// Use preset as chain Config
    #[clap(env = "MADARA_PRESET", long, value_name = "PRESET NAME", group = "chain_config")]
    pub preset: Option<ChainPreset>,

    /// Overrides parameters from the Chain Config.
    #[clap(flatten)]
    pub chain_config_override: ChainConfigOverrideParams,
}

impl RunCmd {
    pub async fn node_name_or_provide(&mut self) -> &str {
        if self.name.is_none() {
            let name = crate::util::get_random_pokemon_name().await.unwrap_or_else(|e| {
                tracing::warn!("Failed to get random pokemon name: {}", e);
                "madara".to_string()
            });

            self.name = Some(name);
        }
        self.name.as_ref().expect("Name was just set")
    }

    pub fn chain_config(&self) -> anyhow::Result<Arc<ChainConfig>> {
        let mut chain_config = match (self.preset.as_ref(), self.chain_config_path.as_ref(), self.devnet) {
            // Read from the preset if provided
            (Some(preset), _, _) => ChainConfig::from(preset),
            // Read the config path if provided
            (_, Some(path), _) => ChainConfig::from_yaml(path).map_err(|err| {
                tracing::error!("Failed to load config from YAML at path '{}': {}", path.display(), err);
                anyhow::anyhow!("Failed to load chain config from file")
            })?,
            // Devnet default preset is Devnet if not provided by CLI
            (_, _, true) => ChainConfig::from(&ChainPreset::Devnet),
            _ => {
                let error_message = if self.is_sequencer() {
                    "In Sequencer mode, you must define a Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`."
                } else {
                    "No network specified. Please provide a network with `--network <NETWORK>` or a custom Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`."
                };
                let preset_info = "\nThe default presets are:\n- 'mainnet' - (configs/presets/mainnet.yaml)\n- 'sepolia' - (configs/presets/sepolia.yaml)\n- 'integration' - (configs/presets/integration.yaml)\n- 'devnet' - (configs/presets/devnet.yaml)";
                return Err(anyhow::anyhow!("{}{}", error_message, preset_info));
            }
        };

        if !self.chain_config_override.overrides.is_empty() {
            chain_config = self.chain_config_override.override_chain_config(chain_config)?;
        };

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
                tracing::error!("{}", "Chain config path is not set");
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

    pub fn is_devnet(&self) -> bool {
        self.devnet
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
    pub fn chain_id(&self) -> ChainId {
        match self {
            NetworkType::Main => ChainId::Mainnet,
            NetworkType::Test => ChainId::Sepolia,
            NetworkType::Integration => ChainId::IntegrationSepolia,
            NetworkType::Devnet => ChainId::Other("MADARA_DEVNET".to_string()),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ChainPreset {
    Mainnet,
    Sepolia,
    Integration,
    Devnet,
}

impl FromStr for ChainPreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mainnet" => Ok(ChainPreset::Mainnet),
            "sepolia" => Ok(ChainPreset::Sepolia),
            "integration" => Ok(ChainPreset::Integration),
            "devnet" => Ok(ChainPreset::Devnet),
            _ => Err(format!("Unknown preset: {}", s)),
        }
    }
}

impl From<&ChainPreset> for ChainConfig {
    fn from(value: &ChainPreset) -> Self {
        match value {
            ChainPreset::Mainnet => ChainConfig::starknet_mainnet(),
            ChainPreset::Sepolia => ChainConfig::starknet_sepolia(),
            ChainPreset::Integration => ChainConfig::starknet_integration(),
            ChainPreset::Devnet => ChainConfig::madara_devnet(),
        }
    }
}
