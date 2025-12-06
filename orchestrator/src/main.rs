// TODO(mohit): Remove this once large error variants are refactored.
// This allow is here to silence `clippy::result_large_err`, which warns when a
// function returns a `Result<T, E>` where `E` is an enum with large data variants.
// Clippy suggests using `Box<Error>` or reducing enum size to avoid bloated
// stack frames. We currently keep the large enums (JobError, QueueError, StorageError)
// unboxed for clarity and easier pattern matching.
// Remove this once we:
//   1. Refactor error enums to smaller variants, OR
//   2. Box the large variants (e.g. `Box<SomeStruct>`), OR
//   3. Switch to an error-handling strategy like `anyhow::Error`.
//
// In short: delete this `#![allow(clippy::result_large_err)]` after
// error types are slimmed down or boxed.
#![allow(clippy::result_large_err)]

use clap::Parser as _;
use dotenvy::dotenv;
use orchestrator::cli::{Cli, Commands, RunCmd, SetupCmd};
use orchestrator::core::client::lock::mongodb::MongoLockClient;
use orchestrator::core::config::Config;
use orchestrator::server::setup_server;
use orchestrator::setup::setup;
use orchestrator::types::params::OTELConfig;
use orchestrator::utils::instrument::OrchestratorInstrumentation;
use orchestrator::utils::logging::init_logging;
use orchestrator::utils::preflight::run_preflight_checks;
use orchestrator::utils::signal_handler::SignalHandler;
use orchestrator::utils::startup_info::{log_detailed_config, log_startup_info};
use orchestrator::worker::initialize_worker;
use orchestrator::{OrchestratorError, OrchestratorResult};
use std::sync::Arc;
use tracing::{debug, error, info};

#[global_allocator]
static A: jemallocator::Jemalloc = jemallocator::Jemalloc;

/// Start the server
#[tokio::main]
async fn main() {
    dotenv().ok();
    init_logging();
    let cli = Cli::parse();

    match &cli.command {
        Commands::Run { run_command } => match run_orchestrator(run_command).await {
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
        },
        Commands::Setup { setup_command } => match setup_orchestrator(setup_command).await {
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
        },
    }
}

/// Initializes the orchestrator with the provided configuration
/// It does the following:
/// 1. Start instrumentation
/// 2. Generate [Config] from [RunCmd]
/// 3. Run pre-flight health checks for all critical resources
/// 4. Starts the server for sending manual requests
/// 5. Initialize worker
/// 6. Setup signal handling for graceful shutdown
async fn run_orchestrator(run_cmd: &RunCmd) -> OrchestratorResult<()> {
    let config = OTELConfig::try_from(run_cmd.instrumentation_args.clone())?;
    let instrumentation = OrchestratorInstrumentation::new(&config)?;

    // Build Config using the new path (preset/config file) or legacy path (CLI args)
    let config = if run_cmd.config_file.is_some() || run_cmd.preset.is_some() {
        // NEW PATH: Load from preset or config file
        info!("Loading configuration from {} mode", if run_cmd.preset.is_some() { "preset" } else { "config file" });
        let orch_config = orchestrator::config::load_config_from_run_cmd(run_cmd)?;
        Arc::new(Config::from_orchestrator_config(&orch_config).await?)
    } else {
        // LEGACY PATH: Load from CLI args (for backward compatibility)
        info!("Loading configuration from CLI args (legacy mode)");
        let settlement_config = orchestrator::types::params::settlement::SettlementConfig::try_from(run_cmd.clone())?;
        let da_config = orchestrator::types::params::da::DAConfig::try_from(run_cmd.clone())?;
        let prover_config = orchestrator::types::params::prover::ProverConfig::try_from(run_cmd.clone())?;
        log_detailed_config(&settlement_config, &da_config, &prover_config);
        Arc::new(Config::from_run_cmd(run_cmd).await?)
    };

    // Log comprehensive startup information
    log_startup_info(&config);

    // Run pre-flight health checks to ensure all dependencies are accessible
    run_preflight_checks(config.database(), config.storage(), config.queue(), config.alerts()).await?;

    // Run the server in a separate tokio spawn task
    setup_server(config.clone()).await?;

    debug!("Application router initialized");

    info!("Initializing MongoDB lock client");
    let lock_client = MongoLockClient::from_run_cmd(run_cmd.clone()).await?;
    lock_client.initialize().await.map_err(|e| OrchestratorError::SetupError(e.to_string()))?;
    info!("MongoDB lock client initialize successfully");

    // Set up comprehensive signal handling for Docker/Kubernetes
    info!("Setting up signal handler for graceful shutdown");
    let mut signal_handler = SignalHandler::new();
    let shutdown_token = signal_handler.get_shutdown_token();

    // Initialize workers and keep the controller for shutdown
    let worker_controller = initialize_worker(config.clone(), shutdown_token).await?;

    let shutdown_signal = signal_handler.wait_for_shutdown().await;

    info!("Initiating orchestrator shutdown sequence (triggered by: {})", shutdown_signal);

    // Perform a graceful shutdown with timeout
    let shutdown_result = signal_handler
        .handle_graceful_shutdown(
            || async {
                // Graceful shutdown for workers
                worker_controller.shutdown().await?;

                // Analytics Shutdown
                instrumentation.shutdown()?;

                info!("All components shutdown successfully");
                Ok(())
            },
            run_cmd.graceful_shutdown_timeout,
        )
        .await;

    match shutdown_result {
        Ok(()) => {
            info!("Orchestrator service shutdown completed gracefully");
            Ok(())
        }
        Err(e) => {
            error!("Orchestrator service shutdown encountered errors: {}", e);
            // Still return Ok to avoid panic, as we tried our best
            Ok(())
        }
    }
}

/// setup_orchestrator - Initializes the orchestrator with the provided configuration
async fn setup_orchestrator(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    setup(setup_cmd).await
}
