pub mod controller;
pub mod event_handler;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use controller::worker_controller::WorkerController;

use crate::{core::config::Config, OrchestratorError, OrchestratorResult};
use std::sync::Arc;

/// Initializes the worker with the provided configuration
///
/// This function initializes the worker with the provided configuration.
/// It starts all workers and returns the controller immediately for shutdown management.
/// The function should be called before the worker is started.
pub async fn initialize_worker(config: Arc<Config>) -> OrchestratorResult<WorkerController> {
    let controller = WorkerController::new(config);
    match controller.run().await {
        Ok(_) => {
            tracing::info!("Workers initialized and started successfully");
            Ok(controller)
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize workers");
            Err(OrchestratorError::EventSystemError(e))
        }
    }
}
