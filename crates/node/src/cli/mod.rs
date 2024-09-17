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
pub use sync::*;
pub use telemetry::*;

use clap::ArgGroup;
use mp_chain_config::ChainConfig;
use std::path::PathBuf;
use std::sync::Arc;
use url::Url;

/// Madara: High performance Starknet sequencer/full-node.
#[derive(Clone, Debug, clap::Parser)]
#[clap(group(ArgGroup::new("chain_config").args(["chain_config_path", "preset"]).required(true)))]
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

    /// Enable authority mode: the node will run as a sequencer and try and produce its own blocks.
    #[arg(long)]
    pub authority: bool,

    /// The network chain configuration.
    #[clap(long, short, default_value = "main")]
    pub network: NetworkType,

    /// Chain configuration file path.
    #[clap(long, value_name = "CHAIN CONFIG FILE PATH")]
    pub chain_config_path: Option<PathBuf>,

    /// Use preset as chain Config
    #[clap(long, value_name = "PRESET NAME")]
    pub preset: Option<String>,

    /// Allow to override some parameters present in preset or configuration file
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
        let mut chain_config: ChainConfig = match &self.preset {
            Some(preset_name) => ChainConfig::from_preset(preset_name.as_str())?,
            None => {
                ChainConfig::from_yaml(&self.chain_config_path.clone().expect("Failed to retrieve chain config path"))?
            }
        };

        // Override stuff if flag is setted
        if self.chain_config_override {
            chain_config = self.chain_params.override_cfg(chain_config);
        }
        Ok(Arc::new(chain_config))
    }

    pub fn is_authority(&self) -> bool {
        self.authority || self.block_production_params.devnet
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
}
