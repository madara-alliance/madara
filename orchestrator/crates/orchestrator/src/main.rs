use anyhow::{Context, Result};
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
use orchestrator::OrchestratorError;
use orchestrator::OrchestratorResult;
use std::sync::Arc;
use tracing::{debug, error, info, instrument};

#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();
    info!("Starting orchestrator");
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { run_command } => {
            info!("Executing run command with args: {:?}", run_command);
            match run_orchestrator(run_command).await {
                Ok(_) => {
                    info!("Orchestrator service started successfully");
                }
                Err(e) => {
                    error!(
                        error = %e,
                        error_chain = ?e,
                        "Failed to start orchestrator service"
                    );
                    panic!("Failed to start orchestrator service: {}", e);
                }
            }
        }
        Commands::Setup { setup_command } => {
            info!("Executing setup command with args: {:?}", setup_command);
            match setup_orchestrator(setup_command).await {
                Ok(_) => {
                    info!("Orchestrator setup completed successfully");
                }
                Err(e) => {
                    error!(
                        error = %e,
                        error_chain = ?e,
                        "Failed to setup orchestrator"
                    );
                    panic!("Failed to setup orchestrator: {}", e);
                }
            }
        }
    }
}

async fn run_orchestrator(run_cmd: &RunCmd) -> Result<()> {
    let config = OTELConfig::try_from(run_cmd.instrumentation_args.clone()).context("Failed to create OTEL config")?;
    let instrumentation = OrchestratorInstrumentation::new(&config).context("Failed to initialize instrumentation")?;
    info!("Starting orchestrator service");

    let config = Arc::new(Config::from_run_cmd(run_cmd).await.context("Failed to create config from run command")?);

    // Run the server in a separate tokio spawn task
    setup_server(config.clone()).await.context("Failed to setup server")?;

    debug!("Application router initialized");
    initialize_worker(config.clone()).await.context("Failed to initialize worker")?;

    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");

    // Analytics Shutdown
    instrumentation.shutdown()?;
    info!("Orchestrator service shutting down");
    Ok(())
}

/// setup_orchestrator - Initializes the orchestrator with the provided configuration
#[instrument(skip(setup_cmd), fields(args = ?setup_cmd))]
async fn setup_orchestrator(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    setup(setup_cmd).await.map_err(|e| OrchestratorError::from(e))
}
