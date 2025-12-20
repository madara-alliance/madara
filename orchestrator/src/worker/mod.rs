pub mod controller;
pub mod core_worker;
pub mod event_handler;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use core_worker::controller::{create_default_controller, WorkerController};

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Initializes the worker with the provided configuration
///
/// This function initializes workers that poll MongoDB directly for available jobs.
/// Workers use atomic MongoDB operations for job claiming and processing.
///
/// It starts all workers in the background and returns the controller for shutdown management.
/// The function should be called before the worker is started.
///
/// # Arguments
/// * `config` - The configuration for the workers
/// * `shutdown_token` - A cancellation token to signal application shutdown on critical errors
///
/// # Returns
/// * `OrchestratorResult<WorkerController>` - The worker controller
pub async fn initialize_worker(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> OrchestratorResult<WorkerController> {
    info!("Initializing workers with queue-less architecture");

    // Create and start worker controller
    let mut controller = create_default_controller(config, shutdown_token.clone());
    controller.start().await.map_err(|e| anyhow::anyhow!("Failed to start workers: {}", e))?;

    info!("Workers initialized and started successfully");
    Ok(controller)
}
