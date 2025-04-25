pub mod controller;
pub mod event_handler;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use controller::worker_controller::WorkerController;

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;

/// initialize_worker - Initializes the worker with the provided configuration
/// This function initializes the worker with the provided configuration.
/// It is responsible for setting up the worker's environment and resources.
/// The function should be called before the worker is started.
pub async fn initialize_worker(config: Arc<Config>) -> OrchestratorResult<()> {
    let controller = WorkerController::new(config.clone());
    match controller.run().await {
        Ok(_) => tracing::info!("Consumers initialized successfully"),
        Err(e) => {
            tracing::error!(error = %e, "Failed to initialize consumers");
            panic!("Failed to init consumers: {}", e);
        }
    }
    Ok(())
}
