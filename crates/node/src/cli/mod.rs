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

    /// Allow overriding parameters present in preset or configuration file.
    #[clap(long, action = clap::ArgAction::SetTrue, value_name = "OVERRIDE CONFIG FLAG")]
    pub chain_config_override: bool,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub chain_params: ChainConfigOverrideParams,
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

    pub fn get_config(&self) -> anyhow::Result<Arc<ChainConfig>> {
        let chain_config = match &self.preset {
            Some(preset_name) => ChainConfig::from_preset(preset_name.as_str()).map_err(|err| {
                log::error!("Failed to load config from preset '{}': {}", preset_name, err);
                err
            })?,
            None => {
                let path = self.chain_config_path.clone().ok_or_else(|| {
                    log::error!("{}", "Chain config path is not set");
                    if self.is_sequencer() {
                        return anyhow::anyhow!("In Sequencer or Devnet mode, you must define a Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`.\nThe default presets are:\n- 'mainnet' - (crates/primitives/chain_config/presets/mainnet.yml)\n- 'sepolia' - (crates/primitives/chain_config/presets/sepolia.yml)\n- 'integration' - (crates/primitives/chain_config/presets/integration.yml)\n- 'test' - (crates/primitives/chain_config/presets/test.yml)");
                    }
                    else {
                        return anyhow::anyhow!("No network specified. Please provide a network with `--network <NETWORK>` or a custom Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`.\nThe default presets are:\n- 'mainnet' - (crates/primitives/chain_config/presets/mainnet.yml)\n- 'sepolia' - (crates/primitives/chain_config/presets/sepolia.yml)\n- 'integration' - (crates/primitives/chain_config/presets/integration.yml)\n- 'test' - (crates/primitives/chain_config/presets/test.yml)");
                    }
                })?;
                ChainConfig::from_yaml(&path).map_err(|err| {
                    log::error!("Failed to load config from YAML at path '{}': {}", path.display(), err);
                    err
                })?
            }
        };

        // Override stuff if flag is set
        let chain_config =
            if self.chain_config_override { self.chain_params.override_cfg(chain_config) } else { chain_config };

        Ok(Arc::new(chain_config))
    }

    /// Assigns a specific ChainConfig based on a defined network.
    pub fn set_preset_from_network(&self) -> anyhow::Result<Arc<ChainConfig>> {
        let chain_config = match self.network {
            Some(NetworkType::Main) => ChainConfig::starknet_mainnet().map_err(|err| {
                log::error!("Failed to load Starknet Mainnet config: {}", err);
                err
            })?,
            Some(NetworkType::Test) => ChainConfig::starknet_sepolia().map_err(|err| {
                log::error!("Failed to load Starknet Testnet config: {}", err);
                err
            })?,
            Some(NetworkType::Integration) => ChainConfig::starknet_integration().map_err(|err| {
                log::error!("Failed to load Starknet Integration config: {}", err);
                err
            })?,
            Some(NetworkType::Devnet) => ChainConfig::test_config().map_err(|err| {
                log::error!("Failed to load Madara Test Config: {}", err);
                err
            })?,
            None => {
                log::error!("{}", "Chain config path is not set");
                return Err(anyhow::anyhow!("No network specified. Please provide a network with `--network <NETWORK>` or a custom Chain config path with `--chain-config-path <CHAIN CONFIG FILE PATH>` or use a preset with `--preset <PRESET NAME>`"));
            }
        };

        // Override stuff if flag is set
        let chain_config =
            if self.chain_config_override { self.chain_params.override_cfg(chain_config) } else { chain_config };

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

    pub fn to_chain_id(&self) -> ChainId {
        match self {
            NetworkType::Main => ChainId::Mainnet,
            NetworkType::Test => ChainId::Sepolia,
            NetworkType::Integration => ChainId::IntegrationSepolia,
            NetworkType::Devnet => ChainId::Other("MADARA_TEST".to_string()),
        }
    }
}
