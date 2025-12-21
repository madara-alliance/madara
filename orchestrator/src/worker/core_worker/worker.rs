/// Unified Worker - handles both processing and verification phases
///
/// Polls MongoDB for jobs and executes them based on the configured phase.
use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::JobType;
use crate::worker::event_handler::service::JobHandlerService;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

/// The phase of job handling this worker is responsible for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPhase {
    Processing,
    Verification,
}

impl WorkerPhase {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkerPhase::Processing => "processing",
            WorkerPhase::Verification => "verification",
        }
    }
}

/// Unified worker for processing or verifying jobs of a single job type
pub struct Worker {
    job_type: JobType,
    phase: WorkerPhase,
    config: Arc<Config>,
    orchestrator_id: String,
    pub max_concurrent: usize,
    poll_interval_ms: u64,
    shutdown_token: CancellationToken,
}

impl Worker {
    pub fn new(
        job_type: JobType,
        phase: WorkerPhase,
        config: Arc<Config>,
        orchestrator_id: String,
        max_concurrent: usize,
        poll_interval_ms: u64,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self { job_type, phase, config, orchestrator_id, max_concurrent, poll_interval_ms, shutdown_token }
    }

    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("Starting {} worker for {:?}", self.phase.as_str(), self.job_type);
        debug!(
            job_type = ?self.job_type,
            phase = self.phase.as_str(),
            max_concurrent = self.max_concurrent,
            poll_interval_ms = self.poll_interval_ms,
            orchestrator_id = %self.orchestrator_id,
            "Worker config"
        );

        loop {
            if self.shutdown_token.is_cancelled() {
                info!("Stopping {} worker for {:?}", self.phase.as_str(), self.job_type);
                break;
            }

            match self.poll_and_execute().await {
                Ok(false) => {
                    sleep(Duration::from_millis(self.poll_interval_ms)).await;
                }
                Ok(true) => {}
                Err(e) => {
                    error!("Error in {} worker for {:?}: {}", self.phase.as_str(), self.job_type, e);
                    sleep(Duration::from_millis(self.poll_interval_ms)).await;
                }
            }
        }

        Ok(())
    }

    async fn poll_and_execute(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        let current_count =
            self.config.database().count_claimed_jobs_by_type(&self.orchestrator_id, &self.job_type).await?;

        if current_count >= self.max_concurrent as u64 {
            debug!(
                job_type = ?self.job_type,
                phase = self.phase.as_str(),
                current = current_count,
                max = self.max_concurrent,
                "At concurrency limit"
            );
            return Ok(false);
        }

        if let Some(job) = self.try_get_job().await? {
            self.execute_job(&job).await
        } else {
            Ok(false)
        }
    }

    async fn try_get_job(&self) -> Result<Option<JobItem>, Box<dyn std::error::Error + Send + Sync>> {
        let job = match self.phase {
            WorkerPhase::Processing => self.config.database().get_processable_job(&self.job_type).await?,
            WorkerPhase::Verification => self.config.database().get_verifiable_job(&self.job_type).await?,
        };

        if let Some(ref job) = job {
            debug!(
                job_id = %job.id,
                job_type = ?self.job_type,
                phase = self.phase.as_str(),
                status = ?job.status,
                internal_id = %job.internal_id,
                "Found job"
            );
        }

        Ok(job)
    }

    async fn execute_job(&self, job: &JobItem) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        info!("{} job {} for {:?}", self.phase_action(), job.internal_id, self.job_type);

        let result = match self.phase {
            WorkerPhase::Processing => JobHandlerService::process_job(job.id, self.config.clone()).await,
            WorkerPhase::Verification => JobHandlerService::verify_job(job.id, self.config.clone()).await,
        };

        match result {
            Ok(()) => {
                info!("Completed {} for job {}", self.phase.as_str(), job.internal_id);
                Ok(true)
            }
            Err(e) => {
                error!("Failed {} for job {}: {}", self.phase.as_str(), job.internal_id, e);
                Err(Box::new(e))
            }
        }
    }

    fn phase_action(&self) -> &'static str {
        match self.phase {
            WorkerPhase::Processing => "Processing",
            WorkerPhase::Verification => "Verifying",
        }
    }
}
