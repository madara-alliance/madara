/// Worker implementation for queue-less job processing
///
/// This worker actively polls MongoDB for available jobs and processes them
/// using atomic claim operations to prevent race conditions.
use super::config::WorkerConfig;
use super::metrics;
use crate::core::config::Config;
use crate::types::jobs::job_item::JobItem;
use crate::worker::event_handler::service::JobHandlerService;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, trace, warn};

/// Worker state for error handling and circuit breaking
#[derive(Debug)]
struct WorkerState {
    /// Consecutive error count for exponential backoff
    consecutive_errors: u32,
    /// Circuit breaker state
    circuit_open: bool,
    /// Circuit breaker open timestamp
    circuit_open_at: Option<Instant>,
}

impl WorkerState {
    fn new() -> Self {
        Self { consecutive_errors: 0, circuit_open: false, circuit_open_at: None }
    }

    fn reset_errors(&mut self) {
        self.consecutive_errors = 0;
        self.circuit_open = false;
        self.circuit_open_at = None;
    }

    fn increment_error(&mut self, config: &WorkerConfig) {
        self.consecutive_errors += 1;

        // FIX-07: Circuit breaker logic
        if self.consecutive_errors >= config.circuit_breaker_threshold {
            self.circuit_open = true;
            self.circuit_open_at = Some(Instant::now());
            warn!(
                job_type = ?config.job_type,
                consecutive_errors = self.consecutive_errors,
                threshold = config.circuit_breaker_threshold,
                "Circuit breaker opened due to consecutive errors"
            );
        }
    }

    fn should_attempt_close(&self, config: &WorkerConfig) -> bool {
        if let Some(open_at) = self.circuit_open_at {
            let elapsed = open_at.elapsed();
            let reset_timeout = Duration::from_secs(config.circuit_breaker_reset_timeout_secs);
            elapsed >= reset_timeout
        } else {
            false
        }
    }
}

/// Worker for a single job type
pub struct Worker {
    config: WorkerConfig,
    orchestrator_config: Arc<Config>,
    orchestrator_id: String,
    shutdown_token: CancellationToken,
    state: WorkerState,
}

impl Worker {
    /// Create a new worker
    pub fn new(
        config: WorkerConfig,
        orchestrator_config: Arc<Config>,
        orchestrator_id: String,
        shutdown_token: CancellationToken,
    ) -> Self {
        Self { config, orchestrator_config, orchestrator_id, shutdown_token, state: WorkerState::new() }
    }

    /// Run the worker loop
    pub async fn run(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            job_type = ?self.config.job_type,
            poll_interval_ms = self.config.poll_interval_ms,
            max_concurrent_processing = self.config.max_concurrent_processing,
            max_concurrent_verification = self.config.max_concurrent_verification,
            "Starting worker"
        );

        loop {
            // Check for shutdown signal
            if self.shutdown_token.is_cancelled() {
                info!(job_type = ?self.config.job_type, "Worker received shutdown signal");
                break;
            }

            // FIX-07: Circuit breaker check
            if self.state.circuit_open {
                if self.state.should_attempt_close(&self.config) {
                    info!(
                        job_type = ?self.config.job_type,
                        "Circuit breaker reset timeout elapsed, attempting to close"
                    );
                    self.state.reset_errors();
                } else {
                    // Wait before checking again
                    sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                    continue;
                }
            }

            // Try to claim and process a job
            match self.poll_and_process().await {
                Ok(job_processed) => {
                    if job_processed {
                        // Reset error state on successful processing
                        self.state.reset_errors();
                    } else {
                        // No jobs available, record empty poll and wait
                        metrics::record_empty_poll(&self.config.job_type);
                        sleep(Duration::from_millis(self.config.poll_interval_ms)).await;
                    }
                }
                Err(e) => {
                    // FIX-05: Exponential backoff on errors
                    error!(
                        job_type = ?self.config.job_type,
                        error = %e,
                        consecutive_errors = self.state.consecutive_errors + 1,
                        "Error during poll and process"
                    );

                    metrics::record_db_error("poll_and_process", &e.to_string());
                    self.state.increment_error(&self.config);

                    let backoff_delay = self.config.calculate_backoff(self.state.consecutive_errors - 1);
                    warn!(
                        job_type = ?self.config.job_type,
                        backoff_ms = backoff_delay.as_millis(),
                        "Applying exponential backoff"
                    );
                    sleep(backoff_delay).await;
                }
            }
        }

        info!(job_type = ?self.config.job_type, "Worker stopped");
        Ok(())
    }

    /// Poll for a job and process it if available
    ///
    /// Returns Ok(true) if a job was processed, Ok(false) if no jobs available
    async fn poll_and_process(&self) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        // Per-job-type concurrency limit: count jobs of THIS type claimed by THIS orchestrator
        let claimed_count = self
            .orchestrator_config
            .database()
            .count_claimed_jobs_by_type(&self.orchestrator_id, &self.config.job_type)
            .await?;

        // Temporary: Use combined limit until Phase 3 splits workers
        let max_concurrent_total =
            (self.config.max_concurrent_processing + self.config.max_concurrent_verification) as u64;
        if claimed_count >= max_concurrent_total {
            trace!(
                job_type = ?self.config.job_type,
                orchestrator_id = %self.orchestrator_id,
                claimed_count = claimed_count,
                max_concurrent_total = max_concurrent_total,
                "Per-job-type concurrency limit reached, skipping claim"
            );
            return Ok(false);
        }

        // Try to claim a job for processing first (higher priority)
        if let Some(job) = self.try_claim_processing_job().await? {
            self.process_claimed_job(job).await?;
            return Ok(true);
        }

        // Try to claim a job for verification
        if let Some(job) = self.try_claim_verification_job().await? {
            self.verify_claimed_job(job).await?;
            return Ok(true);
        }

        // No jobs available in either phase
        Ok(false)
    }

    /// Try to atomically claim a job for processing
    async fn try_claim_processing_job(&self) -> Result<Option<JobItem>, Box<dyn std::error::Error + Send + Sync>> {
        let start = Instant::now();

        let job = self
            .orchestrator_config
            .database()
            .claim_job_for_processing(&self.config.job_type, &self.orchestrator_id)
            .await?;

        let duration_ms = start.elapsed().as_millis() as f64;
        metrics::record_claim_latency(&self.config.job_type, "processing", duration_ms);

        if let Some(ref job) = job {
            metrics::record_processing_claim_success(&self.config.job_type);
            debug!(
                job_id = %job.id,
                job_type = ?self.config.job_type,
                "Successfully claimed job for processing"
            );
        } else {
            metrics::record_processing_claim_failed(&self.config.job_type);
        }

        Ok(job)
    }

    /// Try to atomically claim a job for verification
    async fn try_claim_verification_job(&self) -> Result<Option<JobItem>, Box<dyn std::error::Error + Send + Sync>> {
        let start = Instant::now();

        let job = self
            .orchestrator_config
            .database()
            .claim_job_for_verification(&self.config.job_type, &self.orchestrator_id)
            .await?;

        let duration_ms = start.elapsed().as_millis() as f64;
        metrics::record_claim_latency(&self.config.job_type, "verification", duration_ms);

        if let Some(ref job) = job {
            metrics::record_verification_claim_success(&self.config.job_type);
            debug!(
                job_id = %job.id,
                job_type = ?self.config.job_type,
                "Successfully claimed job for verification"
            );
        } else {
            metrics::record_verification_claim_failed(&self.config.job_type);
        }

        Ok(job)
    }

    /// Process a claimed job using existing JobHandlerService
    async fn process_claimed_job(&self, job: JobItem) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let job_id = job.id;

        info!(
            job_id = %job_id,
            job_type = ?self.config.job_type,
            internal_id = %job.internal_id,
            "Processing claimed job"
        );

        // Use existing JobHandlerService to process the job
        // This maintains compatibility with existing job processing logic
        match JobHandlerService::process_job(job_id, self.orchestrator_config.clone()).await {
            Ok(()) => {
                info!(
                    job_id = %job_id,
                    job_type = ?self.config.job_type,
                    "Job processed successfully"
                );
                Ok(())
            }
            Err(e) => {
                error!(
                    job_id = %job_id,
                    job_type = ?self.config.job_type,
                    error = %e,
                    "Failed to process job"
                );

                // JobHandlerService already handles everything:
                // - Sets status to Failed (terminal state)
                // - Clears claimed_by atomically
                // - Records failure metadata
                // No need to release claim here - already handled atomically
                metrics::record_claim_release(&self.config.job_type, "processing_failed");

                Err(Box::new(e))
            }
        }
    }

    /// Verify a claimed job using existing JobHandlerService
    async fn verify_claimed_job(&self, job: JobItem) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let job_id = job.id;

        info!(
            job_id = %job_id,
            job_type = ?self.config.job_type,
            internal_id = %job.internal_id,
            "Verifying claimed job"
        );

        // Use existing JobHandlerService to verify the job
        match JobHandlerService::verify_job(job_id, self.orchestrator_config.clone()).await {
            Ok(()) => {
                info!(
                    job_id = %job_id,
                    job_type = ?self.config.job_type,
                    "Job verified successfully"
                );
                metrics::record_claim_release(&self.config.job_type, "verification_complete");
                Ok(())
            }
            Err(e) => {
                error!(
                    job_id = %job_id,
                    job_type = ?self.config.job_type,
                    error = %e,
                    "Failed to verify job"
                );

                // JobHandlerService already handles everything:
                // - Sets status to VerificationFailed or Failed (if max retries)
                // - Clears claimed_by atomically
                // - Records failure metadata
                // No need to release claim here - already handled atomically
                metrics::record_claim_release(&self.config.job_type, "verification_failed");

                Err(Box::new(e))
            }
        }
    }

    /// Graceful shutdown - wait for in-flight jobs of this type to complete
    pub async fn shutdown_gracefully(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!(
            job_type = ?self.config.job_type,
            timeout_secs = self.config.shutdown_timeout.as_secs(),
            "Starting graceful shutdown"
        );

        let start = Instant::now();
        let timeout = self.config.shutdown_timeout;

        loop {
            // Check how many jobs of THIS type are still claimed by this orchestrator
            let claimed_count = self
                .orchestrator_config
                .database()
                .count_claimed_jobs_by_type(&self.orchestrator_id, &self.config.job_type)
                .await?;

            if claimed_count == 0 {
                info!(
                    job_type = ?self.config.job_type,
                    "All in-flight jobs completed, shutdown successful"
                );
                return Ok(());
            }

            if start.elapsed() >= timeout {
                warn!(
                    job_type = ?self.config.job_type,
                    remaining_jobs = claimed_count,
                    timeout_secs = timeout.as_secs(),
                    "Graceful shutdown timeout reached, forcing shutdown"
                );
                return Ok(());
            }

            debug!(
                job_type = ?self.config.job_type,
                remaining_jobs = claimed_count,
                elapsed_secs = start.elapsed().as_secs(),
                "Waiting for in-flight jobs to complete"
            );

            sleep(Duration::from_secs(5)).await;
        }
    }
}
