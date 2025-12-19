pub mod controller;
pub mod event_handler;
pub mod greedy;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use controller::worker_controller::WorkerController;
use greedy::controller::{create_default_greedy_controller, GreedyWorkerController};

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Worker mode enum to track which worker system is active
pub enum WorkerMode {
    Sqs(WorkerController),
    Greedy(GreedyWorkerController),
}

/// Initializes the worker with the provided configuration
///
/// This function initializes the worker with the provided configuration based on greedy_mode flag.
/// - If greedy_mode is enabled: starts greedy workers that poll MongoDB directly
/// - If greedy_mode is disabled: starts SQS-based workers (legacy mode)
///
/// It starts all workers in the background and returns the controller for shutdown management.
/// The function should be called before the worker is started.
///
/// # Arguments
/// * `config` - The configuration for the workers
/// * `shutdown_token` - A cancellation token to signal application shutdown on critical errors
///
/// # Returns
/// * `OrchestratorResult<WorkerMode>` - The worker mode enum containing the appropriate controller
pub async fn initialize_worker(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> OrchestratorResult<WorkerMode> {
    let greedy_mode = config.service_config().greedy_mode;

    if greedy_mode {
        info!("Initializing in GREEDY MODE (queue-less architecture)");

        // Create and start greedy worker controller
        let mut greedy_controller = create_default_greedy_controller(config, shutdown_token.clone());
        greedy_controller.start().await.map_err(|e| anyhow::anyhow!("Failed to start greedy workers: {}", e))?;

        info!("Greedy workers initialized and started successfully");
        Ok(WorkerMode::Greedy(greedy_controller))
    } else {
        info!("Initializing in SQS MODE (legacy queue-based architecture)");

        let controller = WorkerController::new(config, shutdown_token.clone());

        // Spawn workers in the background - don't wait for them to complete
        let controller_clone = controller.clone();
        let shutdown_token_clone = shutdown_token.clone();

        tokio::spawn(async move {
            match controller_clone.run().await {
                Ok(()) => {
                    warn!("Worker controller completed unexpectedly - this should not happen during normal operation");
                    warn!("Triggering application shutdown to prevent system inconsistency");
                    shutdown_token_clone.cancel();
                }
                Err(e) => {
                    error!(error = %e, "Critical worker controller error - triggering application shutdown");
                    error!("Worker controller failed: {}. This indicates a serious system problem that requires shutdown to prevent data inconsistency.", e);
                    shutdown_token_clone.cancel();
                }
            }
        });

        info!("SQS workers initialized and started successfully");
        Ok(WorkerMode::Sqs(controller))
    }
}
