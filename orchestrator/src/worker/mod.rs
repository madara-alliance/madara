pub mod controller;
pub mod event_handler;
pub mod greedy;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use greedy::controller::{create_default_greedy_controller, GreedyWorkerController};

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
use tracing::info;

/// Initializes the worker with the provided configuration
///
/// This function initializes greedy workers that poll MongoDB directly.
/// Greedy mode is now the default (and only) worker mode.
///
/// It starts all workers in the background and returns the controller for shutdown management.
/// The function should be called before the worker is started.
///
/// # Arguments
/// * `config` - The configuration for the workers
/// * `shutdown_token` - A cancellation token to signal application shutdown on critical errors
///
/// # Returns
/// * `OrchestratorResult<GreedyWorkerController>` - The greedy worker controller
pub async fn initialize_worker(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> OrchestratorResult<GreedyWorkerController> {
    info!("Initializing workers in GREEDY MODE (queue-less architecture)");

    // Create and start greedy worker controller
    let mut greedy_controller = create_default_greedy_controller(config, shutdown_token.clone());
    greedy_controller.start().await.map_err(|e| anyhow::anyhow!("Failed to start greedy workers: {}", e))?;

    info!("Greedy workers initialized and started successfully");
    Ok(greedy_controller)
}
