use std::path::PathBuf;
use std::result::Result as StdResult;
use std::time::Duration;

use deoxys_runtime::SealingMode;
use mc_sync::fetch::fetchers::{fetch_apply_genesis_block, FetchConfig};
use mc_sync::utility::update_config;
use mc_sync::utils::constant::starknet_core_address;
use reqwest::Url;
use sc_cli::{Result, RpcMethods, RunCmd, SubstrateCli};
use serde::{Deserialize, Serialize};
use sp_core::H160;

use crate::cli::Cli;
use crate::service;

/// Available Sealing methods.
#[derive(Debug, Copy, Clone, clap::ValueEnum, Default, Serialize, Deserialize)]
pub enum Sealing {
    /// Seal using rpc method.
    #[default]
    Manual,
    /// Seal when transaction is executed. This mode does not finalize blocks, if you want to
    /// finalize blocks use `--sealing=instant-finality`.
    Instant,
    /// Seal when transaction is executed with finalization.
    InstantFinality,
}

impl From<Sealing> for SealingMode {
    fn from(value: Sealing) -> Self {
        match value {
            Sealing::Manual => SealingMode::Manual,
            Sealing::Instant => SealingMode::Instant { finalize: false },
            Sealing::InstantFinality => SealingMode::Instant { finalize: true },
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
            pending_polling_interval: Duration::from_secs(2),
        }
    }
}

fn parse_url(s: &str) -> StdResult<Url, url::ParseError> {
    s.parse()
}

#[derive(Clone, Debug, clap::Args)]
pub struct ExtendedRunCmd {
    #[clap(flatten)]
    pub base: RunCmd,

    /// Choose sealing method.
    #[clap(long, value_enum, ignore_case = true)]
    pub sealing: Option<Sealing>,

    /// The L1 rpc endpoint url for state verification
    #[clap(long, value_parser = parse_url)]
    pub l1_endpoint: Option<Url>,

    /// The block you want to start syncing from.
    #[clap(long)]
    pub starting_block: Option<u32>,

    /// The network type to connect to.
    #[clap(long, short, default_value = "integration")]
    pub network: NetworkType,

    /// When enabled, more information about the blocks and their transaction is cached and stored
    /// in the database.
    ///
    /// This may improve response times for RPCs that require that information, but it also
    /// increases the memory footprint of the node.
    #[clap(long)]
    pub cache: bool,

    /// This will invoke sound interpreted from the block hashes.
    #[clap(long)]
    pub sound: bool,

    /// This wrap a specific deoxys environment for a node quick start.
    #[clap(long)]
    pub deoxys: bool,

    /// Disable root verification
    #[clap(long)]
    pub disable_root: bool,

    /// Gateway api key to avoid rate limiting (optional)
    #[clap(long)]
    pub gateway_key: Option<String>,

    /// Polling interval, in seconds.
    #[clap(long, default_value = "2")]
    pub pending_polling_interval: u64,

    /// A flag to run the TUI dashboard
    #[cfg(feature = "tui")]
    #[clap(long)]
    pub tui: bool,
}

pub fn run_node(mut cli: Cli) -> Result<()> {
    #[cfg(feature = "tui")]
    {
        deoxys_tui::modify_substrate_sources();
        if cli.run.tui {
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async { deoxys_tui::run("/tmp/deoxys").await.unwrap() });
                std::process::exit(0)
            });
        }
    }
    if cli.run.base.shared_params.dev {
        override_dev_environment(&mut cli.run);
    } else if cli.run.deoxys {
        deoxys_environment(&mut cli.run);
    }

    let runner = cli.create_runner(&cli.run.base)?;

    // TODO: verify that the l1_endpoint is valid
    let l1_endpoint = if let Some(url) = cli.run.l1_endpoint {
        url
    } else {
        return Err(sc_cli::Error::Input(
            "Missing required --l1-endpoint argument please reffer to https://deoxys-docs.kasar.io".to_string(),
        ));
    };

    runner.run_node_until_exit(|config| async move {
        let sealing = cli.run.sealing.map(Into::into).unwrap_or_default();
        let cache = cli.run.cache;
        let starting_block = cli.run.starting_block;
        let mut fetch_block_config = cli.run.network.block_fetch_config();
        fetch_block_config.sound = cli.run.sound;
        fetch_block_config.verify = !cli.run.disable_root;
        fetch_block_config.api_key = cli.run.gateway_key.clone();
        fetch_block_config.pending_polling_interval = Duration::from_secs(cli.run.pending_polling_interval);
        update_config(&fetch_block_config);

        let genesis_block = fetch_apply_genesis_block(fetch_block_config.clone()).await.unwrap();

        service::new_full(config, sealing, l1_endpoint, cache, fetch_block_config, genesis_block, starting_block)
            .map_err(sc_cli::Error::Service)
    })
}

fn override_dev_environment(cmd: &mut ExtendedRunCmd) {
    // create a reproducible dev environment
    // by disabling the default substrate `dev` behaviour
    cmd.base.shared_params.dev = false;
    cmd.base.shared_params.chain = Some("dev".to_string());

    cmd.base.force_authoring = true;
    cmd.base.alice = true;
    cmd.base.tmp = true;

    // we can't set `--rpc-cors=all`, so it needs to be set manually if we want to connect with external
    // hosts
    cmd.base.rpc_external = true;
    cmd.base.rpc_methods = RpcMethods::Unsafe;
}

fn deoxys_environment(cmd: &mut ExtendedRunCmd) {
    // Set the blockchain network to 'starknet'
    cmd.base.shared_params.chain = Some("starknet".to_string());
    cmd.base.shared_params.base_path.get_or_insert_with(|| PathBuf::from("/tmp/deoxys"));

    // Assign a random pokemon name at each startup
    cmd.base.name.get_or_insert_with(|| {
        tokio::runtime::Runtime::new().unwrap().block_on(mc_sync::utility::get_random_pokemon_name()).unwrap_or_else(
            |e| {
                log::warn!("Failed to get random pokemon name: {}", e);
                "deoxys".to_string()
            },
        )
    });

    // Define telemetry endpoints at starknodes.com
    cmd.base.telemetry_params.telemetry_endpoints = vec![("wss://starknodes.com/submit/".to_string(), 0)];

    // Enables manual sealing for custom block production
    cmd.base.no_grandpa = true;
    cmd.sealing = Some(Sealing::Manual);
}
