/// Worker controller - spawns and manages workers for each job type
use super::config::WorkersConfig;
use super::worker::{Worker, WorkerPhase};
use crate::core::config::Config;
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

pub struct WorkerController {
    config: Arc<Config>,
    workers_config: WorkersConfig,
    shutdown_token: CancellationToken,
    worker_handles: Vec<JoinHandle<()>>,
}

impl WorkerController {
    pub fn new(config: Arc<Config>, workers_config: WorkersConfig, shutdown_token: CancellationToken) -> Self {
        Self { config, workers_config, shutdown_token, worker_handles: Vec::new() }
    }

    pub async fn start(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting worker controller for {} job types", self.workers_config.workers.len());
        debug!(orchestrator_id = %self.workers_config.orchestrator_id, "Controller config");

        let spawn_params: Vec<_> = self
            .workers_config
            .workers
            .iter()
            .flat_map(|(job_type, cfg)| {
                [
                    (job_type.clone(), WorkerPhase::Processing, cfg.max_concurrent_processing, cfg.poll_interval_ms),
                    (
                        job_type.clone(),
                        WorkerPhase::Verification,
                        cfg.max_concurrent_verification,
                        cfg.poll_interval_ms,
                    ),
                ]
            })
            .collect();

        for (job_type, phase, max_concurrent, poll_interval_ms) in spawn_params {
            self.spawn_worker(job_type, phase, max_concurrent, poll_interval_ms);
        }

        info!("Started {} workers", self.worker_handles.len());
        Ok(())
    }

    fn spawn_worker(
        &mut self,
        job_type: crate::types::jobs::types::JobType,
        phase: WorkerPhase,
        max_concurrent: usize,
        poll_interval_ms: u64,
    ) {
        let mut worker = Worker::new(
            job_type,
            phase,
            self.config.clone(),
            self.workers_config.orchestrator_id.clone(),
            max_concurrent,
            poll_interval_ms,
            self.shutdown_token.clone(),
        );

        let handle = tokio::spawn(async move {
            if let Err(e) = worker.run().await {
                error!("Worker error: {}", e);
            }
        });

        self.worker_handles.push(handle);
    }

    pub async fn wait_for_completion(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for handle in self.worker_handles.drain(..) {
            if let Err(e) = handle.await {
                error!("Worker task panicked: {}", e);
            }
        }
        info!("All workers completed");
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Shutting down workers");
        self.shutdown_token.cancel();
        self.wait_for_completion().await
    }
}

/// Create a worker controller with default configuration based on layer
pub fn create_default_controller(config: Arc<Config>, shutdown_token: CancellationToken) -> WorkerController {
    use super::config::WorkerConfig;
    use crate::types::jobs::types::JobType;
    use orchestrator_utils::layer::Layer;

    let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());
    let svc = config.service_config();
    let poll_interval_ms = svc.poll_interval_ms;
    let layer = config.layer();

    let mut workers_config = WorkersConfig::new(orchestrator_id, poll_interval_ms);

    // SNOS jobs - both L2 and L3
    workers_config.add_worker(
        WorkerConfig::new(JobType::SnosRun, poll_interval_ms)
            .with_max_concurrent_processing(svc.max_concurrent_snos_jobs_processing.unwrap_or(10))
            .with_max_concurrent_verification(svc.max_concurrent_snos_jobs_verification.unwrap_or(5)),
    );

    // Proving jobs - both L2 and L3
    workers_config.add_worker(
        WorkerConfig::new(JobType::ProofCreation, poll_interval_ms)
            .with_max_concurrent_processing(svc.max_concurrent_proving_jobs_processing.unwrap_or(5))
            .with_max_concurrent_verification(svc.max_concurrent_proving_jobs_verification.unwrap_or(3)),
    );

    // ProofRegistration - L3 only
    if *layer == Layer::L3 {
        workers_config.add_worker(
            WorkerConfig::new(JobType::ProofRegistration, poll_interval_ms)
                .with_max_concurrent_processing(svc.max_concurrent_aggregator_jobs_processing.unwrap_or(3))
                .with_max_concurrent_verification(svc.max_concurrent_aggregator_jobs_verification.unwrap_or(2)),
        );
    }

    // Aggregator - L2 only
    if *layer == Layer::L2 {
        workers_config.add_worker(
            WorkerConfig::new(JobType::Aggregator, poll_interval_ms)
                .with_max_concurrent_processing(svc.max_concurrent_aggregator_jobs_processing.unwrap_or(3))
                .with_max_concurrent_verification(svc.max_concurrent_aggregator_jobs_verification.unwrap_or(2)),
        );
    }

    // Data submission - L3 only
    if *layer == Layer::L3 {
        workers_config.add_worker(
            WorkerConfig::new(JobType::DataSubmission, poll_interval_ms)
                .with_max_concurrent_processing(svc.max_concurrent_data_submission_jobs_processing.unwrap_or(5))
                .with_max_concurrent_verification(svc.max_concurrent_data_submission_jobs_verification.unwrap_or(3)),
        );
    }

    // State transition - both L2 and L3, always sequential
    workers_config.add_worker(
        WorkerConfig::new(JobType::StateTransition, poll_interval_ms)
            .with_max_concurrent_processing(1)
            .with_max_concurrent_verification(1),
    );

    info!("Created worker controller for {:?} layer", layer);
    debug!(orchestrator_id = %workers_config.orchestrator_id, "Controller ID");

    WorkerController::new(config, workers_config, shutdown_token)
}
