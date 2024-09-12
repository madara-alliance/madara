pub mod block_production;
pub mod db;
pub mod l1;
pub mod prometheus;
pub mod rpc;
pub mod sync;
pub mod telemetry;

use crate::cli::l1::L1SyncParams;
pub use block_production::*;
pub use db::*;
pub use prometheus::*;
pub use rpc::*;
pub use sync::*;
pub use telemetry::*;

use clap::ArgGroup;
use mp_chain_config::{ChainConfig, StarknetVersion};
use primitive_types::H160;
use starknet_api::core::PatriciaKey;
use starknet_api::core::{ChainId, ContractAddress};
use starknet_api::{contract_address, felt, patricia_key};
use starknet_core::types::Felt;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
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

    //Overrideable args
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED CHAIN NAME")]
    pub chain_name: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED CHAIN ID")]
    pub chain_id: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED NATIVE FEE TOKEN")]
    pub native_fee_token_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED PARENT FEE TOKEN")]
    pub parent_fee_token_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED LATEST PROTOCOL VERSION")]
    pub latest_protocol_version: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED BLOCK TIME")]
    pub block_time: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED PENDING BLOCK UPDATE")]
    pub pending_block_update_time: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED SEQUENCER ADDRESS")]
    pub sequencer_address: Option<String>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED MAX NONCE VALIDATION FOR SKIP")]
    pub max_nonce_for_validation_skip: Option<u64>,
    #[arg(long, requires = "chain_config_override", value_name = "OVERRIDED ETH CORE CONTRACT")]
    pub eth_core_contract_address: Option<String>,

    /// Run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
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
        let run_cmd = self.clone();
        if self.chain_config_override {
            (
                chain_config.chain_name,
                chain_config.chain_id,
                chain_config.native_fee_token_address,
                chain_config.parent_fee_token_address,
                chain_config.latest_protocol_version,
                chain_config.block_time,
                chain_config.pending_block_update_time,
                chain_config.sequencer_address,
                chain_config.max_nonce_for_validation_skip,
                chain_config.eth_core_contract_address,
            ) = (
                run_cmd.chain_name.map_or(chain_config.chain_name, |v| v),
                run_cmd.chain_id.map_or(chain_config.chain_id, |v| ChainId::from(v)),
                run_cmd
                    .native_fee_token_address
                    .map_or(chain_config.native_fee_token_address, |v| contract_address!(v.as_str())),
                run_cmd
                    .parent_fee_token_address
                    .map_or(chain_config.parent_fee_token_address, |v| contract_address!(v.as_str())),
                run_cmd.latest_protocol_version.map_or(chain_config.latest_protocol_version, |v| {
                    StarknetVersion::from_str(v.as_str()).expect("failed to retrieve version")
                }),
                run_cmd.block_time.map_or(chain_config.block_time, |v| Duration::from_secs(v)),
                run_cmd
                    .pending_block_update_time
                    .map_or(chain_config.pending_block_update_time, |v| Duration::from_secs(v)),
                run_cmd.sequencer_address.map_or(chain_config.sequencer_address, |v| contract_address!(v.as_str())),
                run_cmd.max_nonce_for_validation_skip.map_or(chain_config.max_nonce_for_validation_skip, |v| v),
                run_cmd.eth_core_contract_address.map_or(chain_config.eth_core_contract_address, |v| {
                    H160::from_str(v.as_str()).expect("failed to parse core contract")
                }),
            );
        };
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
}

impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha-sepolia.starknet.io",
            NetworkType::Integration => "https://integration-sepolia.starknet.io",
        }
    }

    pub fn gateway(&self) -> Url {
        format!("{}/gateway", self.uri()).parse().expect("Invalid uri")
    }

    pub fn feeder_gateway(&self) -> Url {
        format!("{}/feeder_gateway", self.uri()).parse().expect("Invalid uri")
    }
}
