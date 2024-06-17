use std::time::Duration;

use dc_sync::fetch::fetchers::FetchConfig;
use dc_sync::utils::constant::starknet_core_address;
use primitive_types::H160;
use url::Url;

fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct SyncParams {
    /// Disable the sync service. The sync service is responsible for listening for new blocks on starknet and ethereum.
    #[clap(long, alias = "no-sync")]
    pub sync_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// The block you want to start syncing from.
    #[clap(long, value_name = "BLOCK NUMBER")]
    pub starting_block: Option<u64>,

    /// The network to connect to.
    #[clap(long, short, default_value = "main")]
    pub network: NetworkType,

    /// This will produce sound interpreted from the block hashes.
    #[cfg(feature = "m")]
    #[clap(long)]
    pub sound: bool,

    /// Disable state root verification. When importing a block, the state root verification is the most expensive operation.
    /// Disabling it will mean the sync service will have a huge speed-up, at a security cost
    // TODO(docs): explain the security cost
    #[clap(long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional).
    #[clap(long, value_name = "API KEY")]
    pub gateway_key: Option<String>,

    /// Polling interval, in seconds. This only affects the sync service once it has caught up with the blockchain tip.
    #[clap(long, default_value = "2", value_name = "SECONDS")]
    pub sync_polling_interval: u64,

    /// Disable sync polling. This currently means that the sync process will not import any more block once it has caught up with the
    /// blockchain tip.
    #[clap(long)]
    pub no_sync_polling: bool,

    /// Number of blocks to sync. May be useful for benchmarking the sync service.
    #[clap(long, value_name = "NUMBER OF BLOCKS")]
    pub n_blocks_to_sync: Option<u64>,

    /// Periodically create a backup, for debugging purposes. Use it with `--backup-dir <PATH>`.
    #[clap(long, value_name = "NUMBER OF BLOCKS")]
    pub backup_every_n_blocks: Option<u64>,
}

impl SyncParams {
    pub fn block_fetch_config(&self) -> FetchConfig {
        let chain_id = self.network.chain_id();

        let gateway = self.network.gateway();
        let feeder_gateway = self.network.feeder_gateway();
        let l1_core_address = self.network.l1_core_address();

        let polling = if self.no_sync_polling { None } else { Some(Duration::from_secs(self.sync_polling_interval)) };

        #[cfg(feature = "m")]
        let sound = self.sound;
        #[cfg(not(feature = "m"))]
        let sound = false;

        FetchConfig {
            gateway,
            feeder_gateway,
            chain_id,
            sound,
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

    pub fn chain_id(&self) -> starknet_types_core::felt::Felt {
        let tmp = match self {
            NetworkType::Main => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_MAIN").unwrap(),
            NetworkType::Test => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_SEPOLIA").unwrap(),
            NetworkType::Integration => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_INTE").unwrap(),
        };

        starknet_types_core::felt::Felt::from_bytes_be(&tmp.to_bytes_be())
    }

    pub fn l1_core_address(&self) -> H160 {
        match self {
            NetworkType::Main => starknet_core_address::MAINNET.parse().unwrap(),
            NetworkType::Test => starknet_core_address::SEPOLIA_TESTNET.parse().unwrap(),
            NetworkType::Integration => starknet_core_address::SEPOLIA_INTEGRATION.parse().unwrap(),
        }
    }
}
