/// Worker controller for managing multiple job type workers
///
/// Spawns and manages a worker for each configured job type.
use super::config::WorkersConfig;
use super::processing_worker::ProcessingWorker;
use super::verification_worker::VerificationWorker;
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
            "Starting worker controller with split processing/verification workers"
        );

        for (job_type, worker_config) in self.workers_config.workers.iter() {
            let job_type_name = format!("{:?}", job_type);
            let job_type_clone = job_type.clone();

            // Spawn ProcessingWorker for this job type
            let mut processing_worker = ProcessingWorker::new(
                job_type_clone.clone(),
                self.config.clone(),
                self.workers_config.orchestrator_id.clone(),
                worker_config.max_concurrent_processing,
                worker_config.poll_interval_ms,
                self.shutdown_token.clone(),
            );

            let processing_handle = tokio::spawn(async move {
                info!(
                    job_type = %job_type_name,
                    worker_type = "processing",
                    max_concurrent = processing_worker.max_concurrent,
                    "Processing worker starting"
                );

                match processing_worker.run().await {
                    Ok(()) => {
                        info!(
                            job_type = %job_type_name,
                            worker_type = "processing",
                            "Processing worker stopped normally"
                        );
                    }
                    Err(e) => {
                        error!(
                            job_type = %job_type_name,
                            worker_type = "processing",
                            error = %e,
                            "Processing worker stopped with error"
                        );
                    }
                }
            });

            self.worker_handles.push(processing_handle);

            // Spawn VerificationWorker for this job type
            let job_type_name = format!("{:?}", job_type);
            let mut verification_worker = VerificationWorker::new(
                job_type_clone,
                self.config.clone(),
                self.workers_config.orchestrator_id.clone(),
                worker_config.max_concurrent_verification,
                worker_config.poll_interval_ms,
                self.shutdown_token.clone(),
            );

            let verification_handle = tokio::spawn(async move {
                info!(
                    job_type = %job_type_name,
                    worker_type = "verification",
                    max_concurrent = verification_worker.max_concurrent,
                    "Verification worker starting"
                );

                match verification_worker.run().await {
                    Ok(()) => {
                        info!(
                            job_type = %job_type_name,
                            worker_type = "verification",
                            "Verification worker stopped normally"
                        );
                    }
                    Err(e) => {
                        error!(
                            job_type = %job_type_name,
                            worker_type = "verification",
                            error = %e,
                            "Verification worker stopped with error"
                        );
                    }
                }
            });

            self.worker_handles.push(verification_handle);
        }

        info!(
            total_workers = self.worker_handles.len(),
            job_types = self.workers_config.workers.len(),
            "All workers started successfully (2 workers per job type: processing + verification)"
        );

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

    // Default concurrency limits if not specified
    const DEFAULT_PROCESSING_LIMIT: usize = 10;
    const DEFAULT_VERIFICATION_LIMIT: usize = 5;

    // SNOS jobs
    let snos_config = WorkerConfig::new(JobType::SnosRun, poll_interval_ms)
        .with_max_concurrent_processing(
            service_config.max_concurrent_snos_jobs_processing.unwrap_or(DEFAULT_PROCESSING_LIMIT),
        )
        .with_max_concurrent_verification(
            service_config.max_concurrent_snos_jobs_verification.unwrap_or(DEFAULT_VERIFICATION_LIMIT),
        );
    workers_config.add_worker(snos_config);

    // Proving jobs
    let proving_config = WorkerConfig::new(JobType::ProofCreation, poll_interval_ms)
        .with_max_concurrent_processing(service_config.max_concurrent_proving_jobs_processing.unwrap_or(5))
        .with_max_concurrent_verification(service_config.max_concurrent_proving_jobs_verification.unwrap_or(3));
    workers_config.add_worker(proving_config);

    // ProofRegistration jobs
    let proof_registration_config = WorkerConfig::new(JobType::ProofRegistration, poll_interval_ms)
        .with_max_concurrent_processing(service_config.max_concurrent_aggregator_jobs_processing.unwrap_or(3))
        .with_max_concurrent_verification(service_config.max_concurrent_aggregator_jobs_verification.unwrap_or(2));
    workers_config.add_worker(proof_registration_config);

    // Aggregator jobs
    let aggregator_config = WorkerConfig::new(JobType::Aggregator, poll_interval_ms)
        .with_max_concurrent_processing(service_config.max_concurrent_aggregator_jobs_processing.unwrap_or(3))
        .with_max_concurrent_verification(service_config.max_concurrent_aggregator_jobs_verification.unwrap_or(2));
    workers_config.add_worker(aggregator_config);

    // Data submission jobs
    let data_submission_config = WorkerConfig::new(JobType::DataSubmission, poll_interval_ms)
        .with_max_concurrent_processing(service_config.max_concurrent_data_submission_jobs_processing.unwrap_or(5))
        .with_max_concurrent_verification(service_config.max_concurrent_data_submission_jobs_verification.unwrap_or(3));
    workers_config.add_worker(data_submission_config);

    // State transition jobs - always sequential (1 at a time for both processing and verification)
    let state_transition_config = WorkerConfig::new(JobType::StateTransition, poll_interval_ms)
        .with_max_concurrent_processing(1)
        .with_max_concurrent_verification(1);
    workers_config.add_worker(state_transition_config);

    info!(
        orchestrator_id = %workers_config.orchestrator_id,
        snos_processing = service_config.max_concurrent_snos_jobs_processing.unwrap_or(DEFAULT_PROCESSING_LIMIT),
        snos_verification = service_config.max_concurrent_snos_jobs_verification.unwrap_or(DEFAULT_VERIFICATION_LIMIT),
        proving_processing = service_config.max_concurrent_proving_jobs_processing.unwrap_or(5),
        proving_verification = service_config.max_concurrent_proving_jobs_verification.unwrap_or(3),
        "Creating worker controller with split processing/verification concurrency limits"
    );

    WorkerController::new(config, workers_config, shutdown_token)
}
