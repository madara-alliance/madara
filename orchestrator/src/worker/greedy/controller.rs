/// Greedy worker controller for managing multiple job type workers
///
/// Spawns and manages a greedy worker for each configured job type.
use super::config::GreedyWorkersConfig;
use super::worker::GreedyWorker;
use crate::core::config::Config;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// Controller for managing greedy workers
pub struct GreedyWorkerController {
    config: Arc<Config>,
    greedy_config: GreedyWorkersConfig,
    shutdown_token: CancellationToken,
    worker_handles: Vec<JoinHandle<()>>,
}

impl GreedyWorkerController {
    /// Create a new greedy worker controller
    pub fn new(config: Arc<Config>, greedy_config: GreedyWorkersConfig, shutdown_token: CancellationToken) -> Self {
        Self { config, greedy_config, shutdown_token, worker_handles: Vec::new() }
    }

    /// Start all greedy workers
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            worker_count = self.greedy_config.workers.len(),
            orchestrator_id = %self.greedy_config.orchestrator_id,
            "Starting greedy worker controller"
        );

        for (job_type, worker_config) in self.greedy_config.workers.iter() {
            let mut worker = GreedyWorker::new(
                worker_config.clone(),
                self.config.clone(),
                self.greedy_config.orchestrator_id.clone(),
                self.shutdown_token.clone(),
            );

            let job_type_name = format!("{:?}", job_type);

            // Spawn worker as a background task
            let handle = tokio::spawn(async move {
                info!(job_type = %job_type_name, "Greedy worker starting");

                match worker.run().await {
                    Ok(()) => {
                        info!(job_type = %job_type_name, "Greedy worker stopped normally");
                    }
                    Err(e) => {
                        error!(
                            job_type = %job_type_name,
                            error = %e,
                            "Greedy worker stopped with error"
                        );
                    }
                }
            });

            self.worker_handles.push(handle);
        }

        info!(worker_count = self.worker_handles.len(), "All greedy workers started successfully");

        Ok(())
    }

    /// Wait for all workers to complete (typically after shutdown signal)
    pub async fn wait_for_completion(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Waiting for all greedy workers to complete");

        for handle in self.worker_handles.drain(..) {
            if let Err(e) = handle.await {
                error!(error = %e, "Worker task panicked");
            }
        }

        info!("All greedy workers completed");
        Ok(())
    }

    /// Trigger shutdown and wait for graceful completion
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Initiating graceful shutdown of greedy workers");

        // Cancel the shutdown token to signal all workers
        self.shutdown_token.cancel();

        // Wait for all workers to complete
        self.wait_for_completion().await?;

        info!("Greedy worker controller shutdown complete");
        Ok(())
    }
}

/// Helper function to create a greedy worker controller with default configuration
pub fn create_default_greedy_controller(
    config: Arc<Config>,
    shutdown_token: CancellationToken,
) -> GreedyWorkerController {
    // Generate unique orchestrator ID
    let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());

    // Get poll interval from service params
    let poll_interval_ms = config.service_config().greedy_poll_interval_ms;

    // Create configuration with all job types
    let greedy_config = GreedyWorkersConfig::new(orchestrator_id, poll_interval_ms).with_all_job_types();

    GreedyWorkerController::new(config, greedy_config, shutdown_token)
}
