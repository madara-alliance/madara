use dc_sync::fetch::fetchers::FetchConfig;
use dc_sync::utils::constant::starknet_core_address;
use dp_block::chain_config::ChainConfig;
use primitive_types::H160;
use starknet_api::core::ChainId;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

fn parse_url(s: &str) -> Result<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct L1SyncParams {
    /// Disable L1 sync.
    #[clap(long, alias = "no-l1-sync")]
    pub sync_l1_disabled: bool,

    /// The L1 rpc endpoint url for state verification.
    #[clap(long, value_parser = parse_url, value_name = "ETHEREUM RPC URL")]
    pub l1_endpoint: Option<Url>,

    /// The network to connect to.
    #[clap(long, short, default_value = "main")]
    pub network: NetworkType,
}

impl L1SyncParams {
    pub fn block_fetch_config(&self) -> ChainId {
        let chain_id = self.network.chain_id();
        chain_id

        // let gateway = self.network.gateway();
        // let feeder_gateway = self.network.feeder_gateway();
        // let l1_core_address = self.network.l1_core_address();
        //
        // // let polling = if self.no_sync_polling { None } else { Some(Duration::from_secs(self.sync_polling_interval)) };
        //
        // #[cfg(feature = "m")]
        // let sound = self.sound;
        // #[cfg(not(feature = "m"))]
        // let sound = false;
        //
        // FetchConfig {
        //     gateway,
        //     feeder_gateway,
        //     chain_id,
        //     sound,
        //     l1_core_address,
        //     // verify: !self.disable_root,
        //     // api_key: self.gateway_key.clone(),
        //     // sync_polling_interval: polling,
        //     // n_blocks_to_sync: self.n_blocks_to_sync,
        //     sync_l1_disabled: self.sync_l1_disabled,
        // }
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

/// Starknet network configuration.
// TODO: move all that into ChainConfig, and probably move chain config in its own primitive crate too?
impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha-sepolia.starknet.io",
            NetworkType::Integration => "https://integration-sepolia.starknet.io",
        }
    }

    pub fn db_chain_info(&self) -> Arc<ChainConfig> {
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

    pub fn chain_id(&self) -> ChainId {
        match self {
            NetworkType::Main => ChainId::Mainnet,
            NetworkType::Test => ChainId::Sepolia,
            NetworkType::Integration => ChainId::IntegrationSepolia,
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
