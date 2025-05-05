use clap::Parser as _;
use dotenvy::dotenv;
use orchestrator::cli::{Cli, Commands, RunCmd, SetupCmd};
use orchestrator::core::config::Config;
use orchestrator::server::setup_server;
use orchestrator::setup::setup;
use orchestrator::types::params::OTELConfig;
use orchestrator::utils::instrument::OrchestratorInstrumentation;
use orchestrator::utils::logging::init_logging;
use orchestrator::worker::initialize_worker;
use orchestrator::OrchestratorResult;
use std::sync::Arc;
use tracing::{debug, error, info};

#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging("orchestrator");
    info!("Starting orchestrator");
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { run_command } => match run_orchestrator(run_command).await {
            Ok(_) => {
                info!("Orchestrator service started successfully");
            }
            Err(e) => {
                error!("Failed to start orchestrator service: {}", e);
            }
        },
        Commands::Setup { setup_command } => match setup_orchestrator(setup_command).await {
            Ok(_) => {
                info!("Orchestrator setup completed successfully");
            }
            Err(e) => {
                error!("Failed to setup orchestrator: {}", e);
            }
        },
    }
}

async fn run_orchestrator(run_cmd: &RunCmd) -> OrchestratorResult<()> {
    let config = OTELConfig::try_from(run_cmd.instrumentation_args.clone())?;
    let instrumentation = OrchestratorInstrumentation::new(&config)?;
    info!("Starting orchestrator service");

    let config = Arc::new(Config::from_run_cmd(run_cmd).await?);
    debug!("Configuration initialized");

    let server_config = config.clone();
    // Run the server in a separate tokio spawn task
    let server_handle = tokio::spawn(async move {
        let _ = setup_server(server_config).await;
    });

    debug!("Application router initialized");
    initialize_worker(config.clone()).await?;

    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");

    tokio::try_join!(server_handle).expect("Failed to join server handle");

    // Analytics Shutdown
    instrumentation.shutdown()?;
    info!("Orchestrator service shutting down");
    Ok(())
}

/// setup_orchestrator - Initializes the orchestrator with the provided configuration
async fn setup_orchestrator(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    setup(setup_cmd).await
}
