use std::path::PathBuf;
use mc_deoxys::l2::fetch_genesis_block;
use reqwest::Url;
use std::result::Result as StdResult;

use madara_runtime::SealingMode;
use sc_cli::{Result, RpcMethods, RunCmd, SubstrateCli};
use sc_service::BasePath;
use serde::{Deserialize, Serialize};

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

/// A possible network type.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum NetworkType {
    /// The main network (mainnet).
    Main,
    /// The test network (testnet).
    Test,
    /// The integration network.
    Integration,
}

impl NetworkType {
    pub fn uri(&self) -> &'static str {
        match self {
            NetworkType::Main => "https://alpha-mainnet.starknet.io",
            NetworkType::Test => "https://alpha4.starknet.io",
            NetworkType::Integration => "https://external.integration.starknet.io",
        }
    }

    pub fn chain_id(&self) -> starknet_core::types::FieldElement {
        match self {
            NetworkType::Main => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_MAIN").unwrap(),
            NetworkType::Test => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_TEST").unwrap(),
            NetworkType::Integration => starknet_core::types::FieldElement::from_byte_slice_be(b"SN_INTE").unwrap(),
        }
    }

    pub fn block_fetch_config(&self) -> mc_deoxys::FetchConfig {
        let uri = self.uri();
        let chain_id = self.chain_id();

        let gateway = format!("{uri}/gateway").parse().unwrap();
        let feeder_gateway = format!("{uri}/feeder_gateway").parse().unwrap();

        mc_deoxys::FetchConfig { gateway, feeder_gateway, chain_id, workers: 5, sound: false }
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
}

impl ExtendedRunCmd {
    pub fn base_path(&self) -> Result<BasePath> {
        Ok(self
            .base
            .shared_params
            .base_path()?
            .unwrap_or_else(|| BasePath::from_project("", "", &<Cli as SubstrateCli>::executable_name())))
    }
}

pub fn run_node(mut cli: Cli) -> Result<()> {
    if cli.run.base.shared_params.dev {
        override_dev_environment(&mut cli.run);
    } else if cli.run.deoxys {
        deoxys_environment(&mut cli.run);
    }
    let runner = cli.create_runner(&cli.run.base)?;

    //TODO: verify that the l1_endpoint is valid
    let l1_endpoint = if let Some(url) = cli.run.l1_endpoint {
        url
    } else {
        return Err(sc_cli::Error::Input("Missing required --l1-endpoint argument please reffer to https://deoxys-docs.kasar.io".to_string()));
    };

    runner.run_node_until_exit(|config| async move {
        let sealing = cli.run.sealing.map(Into::into).unwrap_or_default();
        let cache = cli.run.cache;
        let mut fetch_block_config = cli.run.network.block_fetch_config();
        let genesis_block = fetch_genesis_block(fetch_block_config.clone()).await.unwrap();
        fetch_block_config.sound = cli.run.sound;

        service::new_full(
            config,
            sealing,
            cli.run.base.rpc_port.unwrap(),
            l1_endpoint,
            cache,
            fetch_block_config,
            genesis_block
        )
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
    cmd.base.shared_params.base_path = Some(PathBuf::from("/tmp/deoxys"));

    // Assign a random pokemon name at each startup
    cmd.base.name = Some(tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(mc_deoxys::utility::get_random_pokemon_name())
        .unwrap());

    // Define telemetry endpoints at deoxys.kasar.io
    cmd.base.telemetry_params.telemetry_endpoints = vec![("wss://deoxys.kasar.io/submit/".to_string(), 0)];

    // Enables authoring and manual sealing for custom block production
    cmd.base.force_authoring = true;
    cmd.base.alice = true;
    cmd.sealing = Some(Sealing::Manual);
}
