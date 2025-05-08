use clap::{ArgGroup, Parser, Subcommand};
use cron::event_bridge::AWSEventBridgeCliArgs;
use provider::aws::AWSConfigCliArgs;
use url::Url;

pub use server::ServerCliArgs as ServerParams;
pub use service::ServiceCliArgs as ServiceParams;
use crate::core::config::StarknetVersion;

pub mod alert;
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
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the orchestrator
    Run {
        #[command(flatten)]
        run_command: Box<RunCmd>,
    },
    /// Setup the orchestrator
    Setup {
        #[command(flatten)]
        setup_command: Box<SetupCmd>,
    },
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
        ArgGroup::new("settlement_layer")
            .args(&["settle_on_ethereum", "settle_on_starknet"])
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
        ArgGroup::new("prover")
            .args(&["sharp", "atlantic"])
            .required(true)
            .multiple(false)
    ),
    group(
        ArgGroup::new("da_layer")
            .args(&["da_on_ethereum"])
            .required(true)
            .multiple(false)
    ),
)]
pub struct RunCmd {
    // Provider Config
    #[clap(flatten)]
    pub aws_config_args: AWSConfigCliArgs,

    // Storage
    #[clap(flatten)]
    pub aws_s3_args: storage::aws_s3::AWSS3CliArgs,

    // Queue
    #[clap(flatten)]
    pub aws_sqs_args: queue::aws_sqs::AWSSQSCliArgs,

    // Server
    #[clap(flatten)]
    pub server_args: server::ServerCliArgs,

    // Alert
    #[clap(flatten)]
    pub aws_sns_args: alert::aws_sns::AWSSNSCliArgs,

    // Database
    #[clap(flatten)]
    pub mongodb_args: database::mongodb::MongoDBCliArgs,

    // Data Availability Layer
    #[clap(flatten)]
    pub ethereum_da_args: da::ethereum::EthereumDaCliArgs,

    #[clap(flatten)]
    pub proving_layout_args: prover_layout::ProverLayoutCliArgs,

    // Settlement Layer
    #[clap(flatten)]
    pub ethereum_settlement_args: settlement::ethereum::EthereumSettlementCliArgs,

    #[clap(flatten)]
    pub starknet_settlement_args: settlement::starknet::StarknetSettlementCliArgs,

    // Prover
    #[clap(flatten)]
    pub sharp_args: prover::sharp::SharpCliArgs,

    #[clap(flatten)]
    pub atlantic_args: prover::atlantic::AtlanticCliArgs,

    // SNOS
    #[clap(flatten)]
    pub snos_args: snos::SNOSCliArgs,

    #[arg(env = "MADARA_ORCHESTRATOR_MADARA_RPC_URL", long, required = true)]
    pub madara_rpc_url: Url,

    #[arg(env = "MADARA_ORCHESTRATOR_MADARA_VERSION", long, required = true)]
    pub madara_version: StarknetVersion,

    // Service
    #[clap(flatten)]
    pub service_args: service::ServiceCliArgs,
    #[clap(flatten)]
    pub instrumentation_args: instrumentation::InstrumentationCliArgs,
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

    // Miscellaneous
    #[arg(env = "MADARA_ORCHESTRATOR_SETUP_TIMEOUT", long, default_value = Some("300"))]
    pub timeout: Option<u64>,

    #[arg(env = "MADARA_ORCHESTRATOR_SETUP_RESOURCE_POLL_INTERVAL", long, default_value = Some("5")
    )]
    pub poll_interval: Option<u64>,
}
