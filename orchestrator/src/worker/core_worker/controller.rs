/// Worker controller for managing multiple job type workers
///
/// Spawns and manages a worker for each configured job type.
use super::config::WorkersConfig;
use super::worker::Worker;
use crate::core::config::Config;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// Controller for managing workers
pub struct WorkerController {
    config: Arc<Config>,
    workers_config: WorkersConfig,
    shutdown_token: CancellationToken,
    worker_handles: Vec<JoinHandle<()>>,
}

impl WorkerController {
    /// Create a new worker controller
    pub fn new(config: Arc<Config>, workers_config: WorkersConfig, shutdown_token: CancellationToken) -> Self {
        Self { config, workers_config, shutdown_token, worker_handles: Vec::new() }
    }

    /// Start all workers
    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            worker_count = self.workers_config.workers.len(),
            orchestrator_id = %self.workers_config.orchestrator_id,
            "Starting worker controller"
        );

        for (job_type, worker_config) in self.workers_config.workers.iter() {
            let mut worker = Worker::new(
                worker_config.clone(),
                self.config.clone(),
                self.workers_config.orchestrator_id.clone(),
                self.shutdown_token.clone(),
            );

            let job_type_name = format!("{:?}", job_type);

            // Spawn worker as a background task
            let handle = tokio::spawn(async move {
                info!(job_type = %job_type_name, "Worker starting");

                match worker.run().await {
                    Ok(()) => {
                        info!(job_type = %job_type_name, "Worker stopped normally");
                    }
                    Err(e) => {
                        error!(
                            job_type = %job_type_name,
                            error = %e,
                            "Worker stopped with error"
                        );
                    }
                }
            });

            self.worker_handles.push(handle);
        }

        info!(worker_count = self.worker_handles.len(), "All workers started successfully");

        Ok(())
    }

    /// Wait for all workers to complete (typically after shutdown signal)
    pub async fn wait_for_completion(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Waiting for all workers to complete");

        for handle in self.worker_handles.drain(..) {
            if let Err(e) = handle.await {
                error!(error = %e, "Worker task panicked");
            }
        }

        info!("All workers completed");
        Ok(())
    }

    /// Trigger shutdown and wait for graceful completion
    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Initiating graceful shutdown of workers");

        // Cancel the shutdown token to signal all workers
        self.shutdown_token.cancel();

        // Wait for all workers to complete
        self.wait_for_completion().await?;

        info!("Worker controller shutdown complete");
        Ok(())
    }
}

/// Helper function to create a worker controller with default configuration
pub fn create_default_controller(config: Arc<Config>, shutdown_token: CancellationToken) -> WorkerController {
    use super::config::WorkerConfig;
    use crate::types::jobs::types::JobType;

    // Generate unique orchestrator ID
    let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());

    // Get service config for per-job-type limits
    let service_config = config.service_config();
    let poll_interval_ms = service_config.poll_interval_ms;

    // Create configuration with all job types and their specific concurrency limits
    let mut workers_config = WorkersConfig::new(orchestrator_id, poll_interval_ms);

    // Default concurrency limit if not specified
    const DEFAULT_MAX_CONCURRENT: usize = 10;

    // SNOS jobs
    let snos_config = WorkerConfig::new(JobType::SnosRun, poll_interval_ms)
        .with_max_concurrent_jobs(service_config.max_concurrent_snos_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT));
    workers_config.add_worker(snos_config);

    // Proving jobs
    let proving_config = WorkerConfig::new(JobType::ProofCreation, poll_interval_ms)
        .with_max_concurrent_jobs(service_config.max_concurrent_proving_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT));
    workers_config.add_worker(proving_config);

    // Aggregator jobs (ProofRegistration)
    let aggregator_config = WorkerConfig::new(JobType::ProofRegistration, poll_interval_ms)
        .with_max_concurrent_jobs(service_config.max_concurrent_aggregator_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT));
    workers_config.add_worker(aggregator_config);

    // Data submission jobs
    let data_submission_config = WorkerConfig::new(JobType::DataSubmission, poll_interval_ms)
        .with_max_concurrent_jobs(service_config.max_concurrent_data_submission_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT));
    workers_config.add_worker(data_submission_config);

    // State transition jobs - always sequential (1 at a time)
    let state_transition_config =
        WorkerConfig::new(JobType::StateTransition, poll_interval_ms).with_max_concurrent_jobs(1);
    workers_config.add_worker(state_transition_config);

    info!(
        orchestrator_id = %workers_config.orchestrator_id,
        snos_limit = service_config.max_concurrent_snos_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT),
        proving_limit = service_config.max_concurrent_proving_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT),
        aggregator_limit = service_config.max_concurrent_aggregator_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT),
        data_submission_limit = service_config.max_concurrent_data_submission_jobs.unwrap_or(DEFAULT_MAX_CONCURRENT),
        "Creating worker controller with per-job-type concurrency limits"
    );

    WorkerController::new(config, workers_config, shutdown_token)
}
