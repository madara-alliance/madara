use clap::Parser as _;
use dotenvy::dotenv;
use orchestrator::cli::{Cli, Commands, RunCmd, SetupCmd};
use orchestrator::config::init_config;
use orchestrator::queue::init_consumers;
use orchestrator::routes::setup_server;
use orchestrator::setup::setup_cloud;
use orchestrator::telemetry::{setup_analytics, shutdown_analytics};

/// Start the server
#[tokio::main]
// not sure why clippy gives this error on the latest rust
// version but have added it for now
#[allow(clippy::needless_return)]
async fn main() {
    dotenv().ok();

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

async fn run_orchestrator(run_cmd: &RunCmd) -> color_eyre::Result<()> {
    // Analytics Setup
    let instrumentation_params = run_cmd.validate_instrumentation_params().expect("Invalid instrumentation params");
    let meter_provider = setup_analytics(&instrumentation_params);
    tracing::info!(service = "orchestrator", "Starting orchestrator service");

    color_eyre::install().expect("Unable to install color_eyre");

    // initial config setup
    let config = init_config(run_cmd).await.expect("Config instantiation failed");
    tracing::debug!(service = "orchestrator", "Configuration initialized");

    // initialize the server
    let _ = setup_server(config.clone()).await;

    tracing::debug!(service = "orchestrator", "Application router initialized");

    // init consumer
    match init_consumers(config).await {
        Ok(_) => tracing::info!(service = "orchestrator", "Consumers initialized successfully"),
        Err(e) => {
            tracing::error!(service = "orchestrator", error = %e, "Failed to initialize consumers");
            panic!("Failed to init consumers: {}", e);
        }
    }

    tokio::signal::ctrl_c().await.expect("Failed to listen for ctrl+c");

    // Analytics Shutdown
    shutdown_analytics(meter_provider, &instrumentation_params);
    tracing::info!(service = "orchestrator", "Orchestrator service shutting down");

    Ok(())
}

async fn setup_orchestrator(setup_cmd: &SetupCmd) -> color_eyre::Result<()> {
    setup_cloud(setup_cmd).await.expect("Failed to setup cloud");
    Ok(())
}
