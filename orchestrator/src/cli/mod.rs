use crate::core::config::StarknetVersion;
use crate::types::Layer;
use clap::{ArgGroup, Parser, Subcommand};
use cron::event_bridge::AWSEventBridgeCliArgs;
use provider::aws::AWSConfigCliArgs;
pub use server::ServerCliArgs as ServerParams;
pub use service::ServiceCliArgs as ServiceParams;
use url::Url;

pub mod alert;
pub mod batching;
pub mod cron;
pub mod da;
pub mod database;
pub mod instrumentation;
pub mod prover;
pub mod prover_layout;
pub mod provider;
pub mod queue;
pub mod server;
pub mod service;
pub mod settlement;
pub mod snos;
pub mod storage;

#[derive(Parser, Debug)]
#[command(
    name = "orchestrator",
    about = "Madara Orchestrator - Starknet block processing and proof generation",
    long_about = "Madara Orchestrator coordinates SNOS execution, proving, DA submission, and state updates for Starknet rollups.\n\n\
    Quick Start:\n  \
    orchestrator run --preset local-dev\n\n\
    Available Presets:\n  \
    • local-dev           - Local development environment\n  \
    • l2-sepolia          - L2 deployment on Ethereum Sepolia\n  \
    • l3-starknet-sepolia - L3 deployment on Starknet Sepolia",
    after_help = "Examples:\n  \
    orchestrator run --preset local-dev\n  \
    orchestrator run --config /path/to/config.yaml\n  \
    orchestrator setup --preset local-dev\n\n\
    For more information, visit: https://github.com/madara-alliance/madara"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the orchestrator service
    #[command(long_about = "Start the orchestrator service to process Starknet blocks.\n\n\
        The orchestrator requires either a preset or a config file to run.\n\n\
        Examples:\n  \
        orchestrator run --preset local-dev\n  \
        orchestrator run --config my-config.yaml")]
    Run {
        #[command(flatten)]
        run_command: Box<RunCmd>,
    },
    /// Setup the orchestrator infrastructure
    #[command(long_about = "Initialize cloud infrastructure for the orchestrator.\n\n\
        This command sets up queues, storage, and other required resources.")]
    Setup {
        #[command(flatten)]
        setup_command: Box<SetupCmd>,
    },
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
#[clap(
    group(
        ArgGroup::new("config_source")
            .args(&["config_file", "preset"])
            .required(true)
            .multiple(false)
    )
)]
pub struct RunCmd {
    // ===== CONFIGURATION =====
    /// Path to YAML configuration file
    ///
    /// Provide a custom configuration file for the orchestrator.
    /// This allows you to define all settings in a structured YAML format.
    ///
    /// Example: --config /path/to/my-config.yaml
    #[arg(long, value_name = "PATH", conflicts_with = "preset")]
    pub config_file: Option<std::path::PathBuf>,

    /// Use a built-in preset configuration
    ///
    /// Available presets:
    ///   • local-dev           - Local development environment
    ///   • l2-sepolia          - L2 deployment on Ethereum Sepolia
    ///   • l3-starknet-sepolia - L3 deployment on Starknet Sepolia
    ///
    /// Example: --preset local-dev
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "config_file",
        value_parser = clap::builder::PossibleValuesParser::new(["local-dev", "l2-sepolia", "l3-starknet-sepolia"])
    )]
    pub preset: Option<String>,

    // ===== INTERNAL USE ONLY - DO NOT USE DIRECTLY =====
    // These fields are kept temporarily for internal config building.
    // They will be removed once the new config system is fully integrated.
    // Users should ONLY use --config or --preset.
    #[clap(flatten, next_help_heading = None)]
    pub mongodb_args: database::mongodb::MongoDBCliArgs,

    #[arg(hide = true)]
    pub madara_rpc_url: Option<Url>,

    #[arg(hide = true)]
    pub madara_feeder_gateway_url: Option<Url>,

    #[arg(hide = true)]
    pub bouncer_weights_limit_file: Option<std::path::PathBuf>,

    #[arg(hide = true)]
    pub layer: Option<Layer>,

    #[arg(hide = true)]
    pub madara_version: Option<StarknetVersion>,

    #[clap(flatten, next_help_heading = None)]
    pub snos_args: snos::SNOSCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub batching_args: batching::BatchingCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub service_args: service::ServiceCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub server_args: server::ServerCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub proving_layout_args: prover_layout::ProverLayoutCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub ethereum_settlement_args: settlement::ethereum::EthereumSettlementCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub starknet_settlement_args: settlement::starknet::StarknetSettlementCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub sharp_args: prover::sharp::SharpCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub atlantic_args: prover::atlantic::AtlanticCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub ethereum_da_args: da::ethereum::EthereumDaCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub starknet_da_args: da::starknet::StarknetDaCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub instrumentation_args: instrumentation::InstrumentationCliArgs,

    #[arg(skip)]
    pub mock_atlantic_server: bool,

    #[arg(skip)]
    pub store_audit_artifacts: bool,

    #[arg(skip)]
    pub graceful_shutdown_timeout: u64,

    #[clap(flatten, next_help_heading = None)]
    pub aws_config_args: AWSConfigCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub aws_s3_args: storage::aws_s3::AWSS3CliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub aws_sqs_args: queue::aws_sqs::AWSSQSCliArgs,

    #[clap(flatten, next_help_heading = None)]
    pub aws_sns_args: alert::aws_sns::AWSSNSCliArgs,
}

#[derive(Parser, Debug, Clone)]
#[command(author, version, about, long_about = None)]
#[clap(
    group(
        ArgGroup::new("provider")
            .args(&["aws"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("storage")
            .args(&["aws_s3"])
            .required(true)
            .multiple(false)
            .requires("provider")
    ),
    group(
      ArgGroup::new("queue")
          .args(&["aws_sqs"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
      ArgGroup::new("alert")
          .args(&["aws_sns"])
          .required(true)
          .multiple(false)
          .requires("provider")
    ),
    group(
        ArgGroup::new("cron")
            .args(&["aws_event_bridge"])
            .required(true)
            .multiple(false)
            .requires("provider")
    ),
)]
pub struct SetupCmd {
    #[arg(env = "MADARA_ORCHESTRATOR_LAYER", long, default_value = "l2", value_enum)]
    pub layer: Layer,

    // AWS Config
    #[clap(flatten)]
    pub aws_config_args: AWSConfigCliArgs,

    // Storage
    #[clap(flatten)]
    pub aws_s3_args: storage::aws_s3::AWSS3CliArgs,

    // Queue
    #[clap(flatten)]
    pub aws_sqs_args: queue::aws_sqs::AWSSQSCliArgs,

    // Alert
    #[clap(flatten)]
    pub aws_sns_args: alert::aws_sns::AWSSNSCliArgs,

    // Cron
    #[clap(flatten)]
    pub aws_event_bridge_args: AWSEventBridgeCliArgs,

    // Database
    #[clap(flatten)]
    pub mongodb_args: database::mongodb::MongoDBCliArgs,

    // Miscellaneous
    #[arg(env = "MADARA_ORCHESTRATOR_SETUP_TIMEOUT", long, default_value = Some("300"))]
    pub timeout: Option<u64>,

    #[arg(env = "MADARA_ORCHESTRATOR_SETUP_RESOURCE_POLL_INTERVAL", long, default_value = Some("5"))]
    pub poll_interval: Option<u64>,
}
