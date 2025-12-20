pub mod controller;
pub mod event_handler;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use controller::worker_controller::WorkerController;

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

/// Initializes the worker with the provided configuration
///
/// This function initializes the worker with the provided configuration.
/// It starts all workers in the background and returns the controller for shutdown management.
/// The function should be called before the worker is started.
///
/// # Arguments
/// * `config` - The configuration for the workers
/// * `shutdown_token` - A cancellation token to signal application shutdown on critical errors
///
/// # Returns
/// * `OrchestratorResult<WorkerController>` - The worker controller for managing shutdown
pub async fn initialize_worker(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> OrchestratorResult<WorkerController> {
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
                error!(error = %e, "🚨 Critical worker controller error - triggering application shutdown");
                error!("Worker controller failed: {}. This indicates a serious system problem that requires shutdown to prevent data inconsistency.", e);
                shutdown_token_clone.cancel();
            }
        }
    });

    tracing::info!("Workers initialized and started successfully");
    Ok(controller)
}
