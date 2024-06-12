use std::path::PathBuf;
use std::time::Duration;

use dc_sync::fetch::fetchers::{fetch_apply_genesis_block, FetchConfig};
use dc_sync::utility::set_config;
use dc_sync::utils::constant::starknet_core_address;
use reqwest::Url;
use sc_cli::{RunCmd, SubstrateCli};
use sp_core::H160;

use crate::cli::Cli;
use crate::service;

/// Starknet network types.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum NetworkType {
    /// The main network (mainnet).
    Main,
    /// The test network (testnet).
    Test,
    /// The integration network.
    Integration,
}

/// Starknet network configuration.
impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha-sepolia.starknet.io",
            NetworkType::Integration => "https://external.integration.starknet.io",
        }
    }

    pub fn chain_id(&self) -> starknet_core::types::FieldElement {
        match self {
            NetworkType::Main => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_MAIN").unwrap(),
            NetworkType::Test => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_SEPOLIA").unwrap(),
            NetworkType::Integration => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_INTE").unwrap(),
        }
    }

    pub fn l1_core_address(&self) -> H160 {
        match self {
            NetworkType::Main => starknet_core_address::MAINNET.parse().unwrap(),
            NetworkType::Test => starknet_core_address::SEPOLIA_TESTNET.parse().unwrap(),
            NetworkType::Integration => starknet_core_address::SEPOLIA_INTEGRATION.parse().unwrap(),
        }
    }

    pub fn block_fetch_config(&self) -> FetchConfig {
        let uri = self.uri();
        let chain_id = self.chain_id();

        let gateway = format!("{uri}/gateway").parse().unwrap();
        let feeder_gateway = format!("{uri}/feeder_gateway").parse().unwrap();
        let l1_core_address = self.l1_core_address();

        FetchConfig {
            gateway,
            feeder_gateway,
            chain_id,
            workers: 5,
            sound: false,
            l1_core_address,
            verify: true,
            api_key: None,
            sync_polling_interval: Some(Duration::from_secs(2)),
            n_blocks_to_sync: None,
        }
    }
}

fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct ExtendedRunCmd {
    #[clap(flatten)]
    pub base: RunCmd,

    /// The L1 rpc endpoint url for state verification
    #[clap(long, value_parser = parse_url)]
    pub l1_endpoint: Option<Url>,

    /// The block you want to start syncing from.
    #[clap(long)]
    pub starting_block: Option<u32>,

    /// The network type to connect to.
    #[clap(long, short, default_value = "integration")]
    pub network: NetworkType,

    /// This will invoke sound interpreted from the block hashes.
    #[clap(long)]
    pub sound: bool,

    /// Disable root verification
    #[clap(long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional)
    #[clap(long)]
    pub gateway_key: Option<String>,

    /// Polling interval, in seconds
    #[clap(long, default_value = "2")]
    pub sync_polling_interval: u64,

    /// Stop sync polling
    #[clap(long, default_value = "false")]
    pub no_sync_polling: bool,

    /// Number of blocks to sync
    #[clap(long)]
    pub n_blocks_to_sync: Option<u64>,

    #[clap(long)]
    pub backup_every_n_blocks: Option<usize>,

    #[clap(long)]
    pub backup_dir: Option<PathBuf>,

    #[clap(long, default_value = "false")]
    pub restore_from_latest_backup: bool,
}