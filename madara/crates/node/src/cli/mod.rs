pub mod analytics;
pub mod block_production;
pub mod chain_config_overrides;
pub mod db;
pub mod gateway;
pub mod l1;
pub mod l2;
pub mod rpc;
pub mod telemetry;
use crate::cli::l1::L1SyncParams;
use analytics::AnalyticsParams;
use anyhow::Context;
pub use block_production::*;
pub use chain_config_overrides::*;
pub use db::*;
pub use gateway::*;
pub use l2::*;
pub use rpc::*;
use std::str::FromStr;
pub use telemetry::*;

use clap::ArgGroup;
use mp_chain_config::ChainConfig;
use std::path::PathBuf;
use std::sync::Arc;

/// Combines multiple cli args into a single easy to use preset
///
/// Some args configurations are getting pretty lengthy and easy to get wrong.
/// [ArgsPresetParams] tries to fix this:
///
/// 1. Argument presets are evaluated _after_ user inputs are parsed. This means
///    it is not possible for users to override cli args set by a preset. This
///    is a limitation in the way in which [clap] currently works.
///
/// 2. Argument presets are evaluated with [RunCmd::apply_arg_preset]. This has
///    to be called manually, or presets will not be applied! All this does is
///    set various cli flags to some predefined sensible value, tailoring
///    towards a general use case.
///
/// # TODO
///
/// ## User input precedence
///
/// This is still very basic. Some nice improvements would be to allow arg
/// presets to be evaluated _before_ the rest of the user input, so for example
/// if an arg preset overrides `--some-arg`, then the user is still able to set
/// its value by specifying`--some-arg` themselves. This would allow for arg
/// presets to be used as the base for more complex user setups.
///
/// ## Configuration file
///
/// Another nice improvement would be the addition of _configuration files_ for
/// a reproducible, declarative setup. This is still a work in progress as of
/// [#285] and is a bit complicated due to this not being a core feature of
/// [clap]. The main issue in this case is that we want the user to be able to
/// pass a configuration file and ignore the rest of the cli arguments _if they
/// are set in the file_. This is non-trivial with the way in which [clap]
/// works (we still want good auto-generated docs and derive api support!).
///
/// [#285]: https://github.com/madara-alliance/madara/issues/285
#[derive(Clone, Debug, clap::Parser)]
#[clap(
    group(
        ArgGroup::new("args-preset")
            .args(&["warp_update_sender", "warp_update_receiver", "gateway", "rpc"])
            .multiple(false)
    )
)]
pub struct ArgsPresetParams {
    /// Sets up the node as a local feeder gateway, stopping any further sync.
    /// This is used to rapidly synchronize local state onto another node with
    /// --warp-update-receiver. You can use this to rapidly migrate to a new
    /// version of madara without having to re-synchronize from genesis.
    #[clap(env = "MADARA_WARP_UPDATE", long, value_name = "WARP UPDATE", group = "args-preset")]
    pub warp_update_sender: bool,

    /// Sets up the node to rapidly synchronize state from a local feeder
    /// gateway. The node and the feeder gateway will shutdown once this process
    /// is complete. We assume the state of the feeder gateway is valid, and
    /// therefore we do not re-compute the state root. You can use this to
    /// rapidly migrate to a new version of madara without having to
    /// re-synchronize from genesis. You can launch the local feeder gateway
    /// using --warp-update-sender.
    #[clap(env = "MADARA_WARP_UPDATE", long, value_name = "WARP UPDATE", group = "args-preset")]
    pub warp_update_receiver: bool,

    /// Sets up the node as an externally facing feeder gateway exposed on
    /// 0.0.0.0. Generally speaking, this means the node will be accessible
    /// from the outside world.
    #[clap(env = "MADARA_GATEWAY", long, value_name = "GATEWAY", group = "args-preset")]
    pub gateway: bool,

    /// Sets up the node as an externally facing rpc provider exposed on
    /// 0.0.0.0. Generally speaking, this means the node will be accessible
    /// from the outside world. Admin rpc methods are also enabled, but are only
    /// exposed on localhost.
    #[clap(env = "MADARA_RPC", long, value_name = "RPC", group = "args-preset")]
    pub rpc: bool,
}

impl ArgsPresetParams {
    pub fn greet(&self) {
        if self.warp_update_sender {
            tracing::info!("💫 Running Warp Update Sender preset")
        } else if self.warp_update_receiver {
            tracing::info!("💫 Running Warp Update Receiver preset")
        } else if self.gateway {
            tracing::info!("💫 Running Gateway preset")
        } else if self.rpc {
            tracing::info!("💫 Running Rpc preset")
        }
    }
}

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
    pub args_preset: ArgsPresetParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub db_params: DbParams,

    #[allow(missing_docs)]
    #[clap(flatten)]
    pub l2_sync_params: L2SyncParams,

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

    /// Allows a devnet to have SN_MAIN or SN_SEPOLIA as its chain ID.
    #[arg(env = "MADARA_DEVNET_UNSAFE", long, requires = "devnet")]
    pub devnet_unsafe: bool,

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
    #[allow(missing_docs)]
    #[clap(flatten)]
    pub chain_config_override: ChainConfigOverrideParams,
}

impl RunCmd {
    // NOTE: (trantorian) I am not entirely satisfied with how this works. The
    // main issue is that users cannot override presets as this resolves _after_
    // all arguments have been assigned. It might be worth forking clap for a
    // better UX.
    pub fn apply_arg_preset(mut self) -> Self {
        if self.args_preset.warp_update_sender {
            self.gateway_params.feeder_gateway_enable = true;
            self.gateway_params.gateway_port = self.l2_sync_params.warp_update_port_fgw;
            self.rpc_params.rpc_admin = true;
            self.rpc_params.rpc_admin_port = self.l2_sync_params.warp_update_port_rpc;
        } else if self.args_preset.gateway {
            self.gateway_params.feeder_gateway_enable = true;
            self.gateway_params.gateway_enable = true;
            self.gateway_params.gateway_external = true;
        } else if self.args_preset.rpc {
            self.rpc_params.rpc_admin = true;
            self.rpc_params.rpc_external = true;
            self.rpc_params.rpc_cors = Some(Cors::All);
        }

        self
    }

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
            (_, Some(path), _) => ChainConfig::from_yaml(path)
                .with_context(|| format!("Failed to load config from YAML at path '{}'", path.display()))?,
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
