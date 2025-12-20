pub mod controller;
pub mod core_worker;
pub mod event_handler;
pub mod parser;
pub mod service;
pub mod traits;
pub mod utils;

use controller::worker_controller::WorkerController as WorkerTriggerController;
use core_worker::controller::{create_default_controller, WorkerController};

use crate::{core::config::Config, OrchestratorResult};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// Combined worker controller that manages both:
/// 1. Core workers (MongoDB job polling)
/// 2. WorkerTrigger controller (SQS queue for batching triggers)
pub struct CombinedWorkerController {
    core_controller: WorkerController,
    trigger_controller: WorkerTriggerController,
    trigger_handle: Option<JoinHandle<()>>,
}

impl CombinedWorkerController {
    /// Create a new combined worker controller
    pub fn new(core_controller: WorkerController, trigger_controller: WorkerTriggerController) -> Self {
        Self { core_controller, trigger_controller, trigger_handle: None }
    }

    /// Start the WorkerTrigger controller in the background
    pub async fn start_trigger_controller(&mut self) {
        let controller = self.trigger_controller.clone();
        self.trigger_handle = Some(tokio::spawn(async move {
            if let Err(e) = controller.run().await {
                error!(error = %e, "WorkerTrigger controller failed");
            }
        }));
        info!("WorkerTrigger controller started for batching triggers");
    }

    /// Shutdown all workers gracefully
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Initiating combined worker controller shutdown");

        // Shutdown core workers
        self.core_controller.shutdown().await?;

        // Shutdown trigger controller
        if let Err(e) = self.trigger_controller.shutdown().await {
            error!(error = %e, "WorkerTrigger controller shutdown failed");
        }

        // Wait for trigger handle to complete
        if let Some(handle) = self.trigger_handle.take() {
            if let Err(e) = handle.await {
                error!(error = %e, "WorkerTrigger task panicked");
            }
        }

        info!("Combined worker controller shutdown complete");
        Ok(())
    }
}

/// Initializes the worker with the provided configuration
///
/// This function initializes:
/// 1. Core workers that poll MongoDB directly for available jobs
/// 2. WorkerTrigger controller that consumes batching triggers from SQS
///
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
/// * `OrchestratorResult<CombinedWorkerController>` - The combined worker controller
pub async fn initialize_worker(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> OrchestratorResult<CombinedWorkerController> {
    info!("Initializing workers with queue-less architecture + WorkerTrigger for batching");

    // Create and start core worker controller (MongoDB polling)
    let mut core_controller = create_default_controller(config.clone(), shutdown_token.clone());
    core_controller.start().await.map_err(|e| anyhow::anyhow!("Failed to start core workers: {}", e))?;

    // Create WorkerTrigger controller (SQS batching triggers)
    let trigger_controller = WorkerTriggerController::new(config, shutdown_token);

    // Combine both controllers
    let mut combined = CombinedWorkerController::new(core_controller, trigger_controller);

    // Start the trigger controller in the background
    combined.start_trigger_controller().await;

    info!("All workers initialized and started successfully");
    Ok(combined)
}
