use std::time::Duration;

use mc_sync::{fetch::fetchers::FetchConfig, utils::constant::starknet_core_address};
use primitive_types::H160;
use url::Url;

fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    #[clap(long)]
    pub sync_disabled: bool,

    /// The L1 rpc endpoint url for state verification. Required.
    #[clap(long, value_parser = parse_url)]
    pub l1_endpoint: Url,

    /// The block you want to start syncing from.
    #[clap(long)]
    pub starting_block: Option<u64>,

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

    /// Disable sync polling
    #[clap(long, default_value = "false")]
    pub no_sync_polling: bool,

    /// Number of blocks to sync
    #[clap(long)]
    pub n_blocks_to_sync: Option<u64>,

    #[clap(long)]
    pub backup_every_n_blocks: Option<u64>,
}

impl SyncParams {
    pub fn block_fetch_config(&self) -> FetchConfig {
        let chain_id = self.network.chain_id();

        let gateway = self.network.gateway();
        let feeder_gateway = self.network.feeder_gateway();
        let l1_core_address = self.network.l1_core_address();

        let polling = if self.no_sync_polling { None } else { Some(Duration::from_secs(self.sync_polling_interval)) };

        FetchConfig {
            gateway,
            feeder_gateway,
            chain_id,
            sound: self.sound,
            l1_core_address,
            verify: !self.disable_root,
            api_key: self.gateway_key.clone(),
            sync_polling_interval: polling,
            n_blocks_to_sync: self.n_blocks_to_sync,
        }
    }
}

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

    pub fn gateway(&self) -> Url {
        format!("{}/gateway", self.uri()).parse().unwrap()
    }

    pub fn feeder_gateway(&self) -> Url {
        format!("{}/feeder_gateway", self.uri()).parse().unwrap()
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
}
