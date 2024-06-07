use std::path::PathBuf;
use std::time::Duration;

use mc_sync::fetch::fetchers::{fetch_apply_genesis_block, FetchConfig};
use mc_sync::utility::set_config;
use mc_sync::utils::constant::starknet_core_address;
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

    /// A flag to run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,

    #[clap(long)]
    pub backup_every_n_blocks: Option<usize>,

    #[clap(long)]
    pub backup_dir: Option<PathBuf>,

    #[clap(long, default_value = "false")]
    pub restore_from_latest_backup: bool,
}

pub fn run_node(mut cli: Cli) -> anyhow::Result<()> {
    // #[cfg(feature = "tui")]
    // {
    //     deoxys_tui::modify_substrate_sources();
    //     if cli.run.tui {
    //         std::thread::spawn(move || {
    //             tokio::runtime::Runtime::new()
    //                 .unwrap()
    //                 .block_on(async { deoxys_tui::run("/tmp/deoxys").await.unwrap() });
    //             std::process::exit(0)
    //         });
    //     }
    // }

    // // Assign a random pokemon name at each startup
    // cli.run.base.name.get_or_insert_with(|| {
    //     tokio::runtime::Runtime::new().unwrap().block_on(mc_sync::utility::get_random_pokemon_name()).unwrap_or_else(
    //         |e| {
    //             log::warn!("Failed to get random pokemon name: {}", e);
    //             "deoxys".to_string()
    //         },
    //     )
    // });

    // deoxys_environment(&mut cli.run);

    // // If --no-telemetry is not set, set the telemetry endpoints to starknodes.com
    // // TODO(merge): telemetry
    // // if !cli.run.base.telemetry_params.no_telemetry {
    // //     cli.run.base.telemetry_params.telemetry_endpoints =
    // // vec![("wss://starknodes.com/submit/".to_string(), 0)]; }

    // // TODO: verify that the l1_endpoint is valid
    // let l1_endpoint = if let Some(url) = cli.run.l1_endpoint {
    //     url
    // } else {
    //     log::error!("Missing required --l1-endpoint argument. The Online documentation is available here: https://deoxys-docs.kasar.io");
    //     return Ok(());
    // };

    // let starting_block = cli.run.starting_block;
    // let mut fetch_block_config = cli.run.network.block_fetch_config();
    // fetch_block_config.sound = cli.run.sound;
    // fetch_block_config.verify = !cli.run.disable_root;
    // fetch_block_config.api_key = cli.run.gateway_key.clone();
    // fetch_block_config.sync_polling_interval =
    //     if cli.run.no_sync_polling { None } else { Some(Duration::from_secs(cli.run.sync_polling_interval)) };
    // fetch_block_config.n_blocks_to_sync = cli.run.n_blocks_to_sync;
    // // unique set of static OnceCell configuration
    // set_config(&fetch_block_config);

    // let genesis_block = fetch_apply_genesis_block(fetch_block_config.clone()).await.unwrap();

    // service::new_full(
    //     config,
    //     sealing,
    //     l1_endpoint,
    //     fetch_block_config,
    //     genesis_block,
    //     starting_block,
    //     cli.run.backup_every_n_blocks,
    //     cli.run.backup_dir,
    //     cli.run.restore_from_latest_backup,
    // )

    todo!()
    // Ok(())
}
