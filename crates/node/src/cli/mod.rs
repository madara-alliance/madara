pub mod block_production;
pub mod db;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

pub use block_production::*;
pub use db::*;
pub use prometheus::*;
pub use rpc::*;
pub use sync::*;
pub use telemetry::*;

use dp_block::chain_config::ChainConfig;
use std::sync::Arc;
use url::Url;

#[derive(Clone, Debug, clap::Parser)]
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

    /// Run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
}

impl RunCmd {
    pub async fn node_name_or_provide(&mut self) -> &str {
        if self.name.is_none() {
            let name = dc_sync::utility::get_random_pokemon_name().await.unwrap_or_else(|e| {
                log::warn!("Failed to get random pokemon name: {}", e);
                "deoxys".to_string()
            });

            self.name = Some(name);
        }
        self.name.as_ref().unwrap()
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
}

impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha-sepolia.starknet.io",
            NetworkType::Integration => "https://integration-sepolia.starknet.io",
        }
    }

    pub fn chain_config(&self) -> Arc<ChainConfig> {
        match self {
            NetworkType::Main => Arc::new(ChainConfig::starknet_mainnet()),
            NetworkType::Test => Arc::new(ChainConfig::starknet_sepolia()),
            NetworkType::Integration => Arc::new(ChainConfig::starknet_integration()),
        }
    }

    pub fn gateway(&self) -> Url {
        format!("{}/gateway", self.uri()).parse().unwrap()
    }

    pub fn feeder_gateway(&self) -> Url {
        format!("{}/feeder_gateway", self.uri()).parse().unwrap()
    }
}
