/// Processing Worker - handles job processing phase
///
/// This worker polls MongoDB for jobs ready to be processed and executes them.
/// It runs independently from the verification worker, with its own concurrency limits.
use super::metrics;
use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::JobType;
use crate::worker::event_handler::service::JobHandlerService;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace};

/// Worker for processing jobs of a single job type
pub struct ProcessingWorker {
    job_type: JobType,
    config: Arc<Config>,
    orchestrator_id: String,
    pub max_concurrent: usize,
    poll_interval_ms: u64,
    shutdown_token: CancellationToken,
}

impl ProcessingWorker {
    /// Create a new processing worker
    pub fn new(
        job_type: JobType,
        config: Arc<Config>,
        orchestrator_id: String,
        max_concurrent: usize,
        poll_interval_ms: u64,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self { job_type, config, orchestrator_id, max_concurrent, poll_interval_ms, shutdown_token }
    }

    /// Run the worker loop
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            job_type = ?self.job_type,
            worker_type = "processing",
            max_concurrent = self.max_concurrent,
            poll_interval_ms = self.poll_interval_ms,
            orchestrator_id = %self.orchestrator_id,
            "Starting processing worker"
        );

        loop {
            // Check for shutdown signal
            if self.shutdown_token.is_cancelled() {
                info!(
                    job_type = ?self.job_type,
                    worker_type = "processing",
                    "Processing worker received shutdown signal"
                );
                break;
            }

            // Try to process a job
            match self.poll_and_process().await {
                Ok(job_processed) => {
                    if !job_processed {
                        // No jobs available, sleep before next poll
                        metrics::record_empty_poll(&self.job_type);
                        sleep(Duration::from_millis(self.poll_interval_ms)).await;
                    }
                }
                Err(e) => {
                    // Error during processing, log and retry
                    error!(
                        job_type = ?self.job_type,
                        worker_type = "processing",
                        error = %e,
                        "Error during poll and process"
                    );
                    metrics::record_db_error("poll_and_process", &e.to_string());
                    sleep(Duration::from_millis(self.poll_interval_ms)).await;
                }
            }
        }

        info!(
            job_type = ?self.job_type,
            worker_type = "processing",
            "Processing worker stopped"
        );
        Ok(())
    }

    /// Poll for a job and process it if available
    ///
    /// Returns Ok(true) if a job was processed, Ok(false) if no jobs available
    async fn poll_and_process(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        info!("poll_and_process is being called");
        // Count current LockedForProcessing jobs for this type claimed by this orchestrator
        let current_count =
            self.config.database().count_claimed_jobs_by_type(&self.orchestrator_id, &self.job_type).await?;

        // Check if we're at the processing concurrency limit
        if current_count >= self.max_concurrent as u64 {
            trace!(
                job_type = ?self.job_type,
                worker_type = "processing",
                current_count = current_count,
                max_concurrent = self.max_concurrent,
                "Processing concurrency limit reached, skipping claim"
            );
            return Ok(false);
        }

        // Try to find a job ready for processing
        if let Some(job) = self.try_get_processable_job().await? {
            // Process the job using JobHandlerService
            match JobHandlerService::process_job(job.id, self.config.clone()).await {
                Ok(()) => {
                    info!(
                        job_id = %job.id,
                        job_type = ?self.job_type,
                        worker_type = "processing",
                        internal_id = %job.internal_id,
                        "Job processed successfully"
                    );
                    Ok(true)
                }
                Err(e) => {
                    error!(
                        job_id = %job.id,
                        job_type = ?self.job_type,
                        worker_type = "processing",
                        error = %e,
                        "Failed to process job"
                    );
                    // JobHandlerService already handles failure states, just return error
                    Err(Box::new(e))
                }
            }
        } else {
            // No jobs available
            Ok(false)
        }
    }

    /// Query for a job ready for processing (without claiming)
    ///
    /// Queries for jobs with status Created or PendingRetryProcessing
    /// The actual claiming happens in JobHandlerService::process_job()
    async fn try_get_processable_job(&self) -> Result<Option<JobItem>, Box<dyn std::error::Error + Send + Sync>> {
        // Use the database's get_processable_job method which queries (but doesn't claim):
        // - Finding jobs with status Created or PendingRetryProcessing
        // - claimed_by = null
        // - available_at = null or <= now
        // The actual atomic claim happens in process_job()
        let job = self.config.database().get_processable_job(&self.job_type).await?;

        if let Some(ref job) = job {
            debug!(
                job_id = %job.id,
                job_type = ?self.job_type,
                worker_type = "processing",
                status = ?job.status,
                "Found job ready for processing"
            );
            metrics::record_processing_claim_success(&self.job_type);
        } else {
            trace!(
                job_type = ?self.job_type,
                worker_type = "processing",
                "No processable jobs found"
            );
            metrics::record_processing_claim_failed(&self.job_type);
        }

        Ok(job)
    }
}
