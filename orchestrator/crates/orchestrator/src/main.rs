use clap::Parser as _;
use dotenvy::dotenv;
use orchestrator::cli::{Cli, Commands, RunCmd, SetupCmd};
use orchestrator::config::init_config;
use orchestrator::core::config::Config;
use orchestrator::params::OTELConfig;
use orchestrator::queue::init_consumers;
use orchestrator::resource::setup::setup;
use orchestrator::routes::setup_server as old_setup_server;
use orchestrator::utils::instrument::OrchestratorInstrumentation;
use orchestrator::utils::logging::init_logging;
use orchestrator::{OrchestratorError, OrchestratorResult};
use tracing::{debug, error, info};

#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

/// Start the server
#[tokio::main]
// not sure why clippy gives this error on the latest rust
// version but have added it for now
#[allow(clippy::needless_return)]
async fn main() {
    dotenv().ok();
    init_logging("orchestrator");
    info!("Starting orchestrator");
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { run_command } => {
            run_orchestrator(run_command).await.expect("Failed to run orchestrator");
        }
        Commands::Setup { setup_command } => {
            setup_orchestrator(setup_command).await.expect("Failed to setup orchestrator");
        }
    }
}

async fn run_orchestrator(run_cmd: &RunCmd) -> OrchestratorResult<()> {
    let config = OTELConfig::try_from(run_cmd.instrumentation_args.clone())?;
    let orchestrator_instrumentation = OrchestratorInstrumentation::setup(&config)?;
    info!("Starting orchestrator service");

    let config = Config::setup(run_cmd).await?;
    // initial config setup
    let old_config = init_config(run_cmd).await.map_err(|e| OrchestratorError::SetupCommandError(e.to_string()))?;
    debug!("Configuration initialized");

    // initialize the server
    let _ = old_setup_server(old_config.clone()).await;

    debug!("Application router initialized");

    // init consumer
    match init_consumers(old_config).await {
        Ok(_) => info!("Consumers initialized successfully"),
        Err(e) => {
            error!(error = %e, "Failed to initialize consumers");
            panic!("Failed to init consumers: {}", e);
        }
    }

    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");

    // Analytics Shutdown
    orchestrator_instrumentation.shutdown()?;
    info!("Orchestrator service shutting down");
    Ok(())
}

/// setup_orchestrator - Initializes the orchestrator with the provided configuration
async fn setup_orchestrator(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    setup(setup_cmd).await
}
