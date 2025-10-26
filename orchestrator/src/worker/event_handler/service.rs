use chrono::Utc;
use futures::FutureExt;
use mockall_double::double;
use opentelemetry::KeyValue;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::jobs::WorkerTriggerType;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::utils::metrics_recorder::MetricsRecorder;
#[double]
use crate::worker::event_handler::factory::factory;
use crate::worker::event_handler::triggers::aggregator::AggregatorJobTrigger;
use crate::worker::event_handler::triggers::batching::BatchingTrigger;
use crate::worker::event_handler::triggers::data_submission_worker::DataSubmissionJobTrigger;
use crate::worker::event_handler::triggers::proof_registration::ProofRegistrationJobTrigger;
use crate::worker::event_handler::triggers::proving::ProvingJobTrigger;
use crate::worker::event_handler::triggers::snos::SnosJobTrigger;
use crate::worker::event_handler::triggers::update_state::UpdateStateJobTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use crate::worker::service::JobService;
use crate::worker::utils::conversion::parse_string;
use tracing::{debug, error, info, warn, Span};

pub struct JobHandlerService;

impl JobHandlerService {
    /// Creates the job in the DB in the created state and adds it to the process queue
    ///
    /// # Arguments
    /// * `job_type` - Type of job to create
    /// * `internal_id` - Unique identifier for internal tracking
    /// * `metadata` - Additional key-value pairs for the job
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # Metrics
    /// * Records block gauge
    /// * Updates successful job operations count
    /// * Records job response time
    ///
    /// # Notes
    /// * Skips creation if the job already exists with the same `internal_id` and `job_type`
    /// * Automatically adds the job to the process queue upon successful creation
    pub async fn create_job(
        job_type: JobType,
        internal_id: String,
        mut metadata: JobMetadata,
        config: Arc<Config>,
    ) -> Result<(), JobError> {
        let start = Instant::now();
        info!(
            log_type = "starting",
            category = "general",
            function_type = "create_job",
            job_type = ?job_type,
            block_no = %internal_id,
            "General create job started for block"
        );

        debug!(
            job_type = ?job_type,
            internal_id = %internal_id,
            metadata = ?metadata,
            "Job creation details"
        );

        let existing_job = config.database().get_job_by_internal_id_and_type(internal_id.as_str(), &job_type).await?;

        if existing_job.is_some() {
            warn!("{}", JobError::JobAlreadyExists { internal_id, job_type });
            return Ok(());
        }

        // Set orchestrator version on job creation
        metadata.common.orchestrator_version = Some(crate::types::constant::ORCHESTRATOR_VERSION.to_string());

        let job_handler = factory::get_job_handler(&job_type).await;
        let job_item = job_handler.create_job(internal_id.clone(), metadata).await?;
        config.database().create_job(job_item.clone()).await?;
        debug!("Job item inside the create job function: {:?}", job_item);

        // Record metrics for job creation
        MetricsRecorder::record_job_created(&job_item);

        // Update job status tracking metrics
        let block_num = parse_string(&internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job_type,
            &JobStatus::Created,
            &job_item.id.to_string(),
        );

        JobService::add_job_to_process_queue(job_item.id, &job_type, config.clone()).await?;

        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("operation_type", "create_job"),
        ];

        info!(
            log_type = "completed",
            category = "general",
            function_type = "create_job",
            block_no = %internal_id,
            "General create job completed for block"
        );

        let duration = start.elapsed();

        // For Aggregator and StateUpdate jobs, fetch the actual block numbers from the batch
        let block_number = match job_type {
            JobType::StateTransition => {
                let batch_number = parse_string(&internal_id)?;

                match config.database().get_aggregator_batches_by_indexes(vec![batch_number as u64]).await {
                    Ok(batches) if !batches.is_empty() => batches[0].end_block as f64,
                    _ => batch_number,
                }
            }
            JobType::Aggregator => {
                let batch_number = parse_string(&internal_id)?;

                // Fetch the batch from the database
                match config.database().get_aggregator_batches_by_indexes(vec![batch_number as u64]).await {
                    Ok(batches) if !batches.is_empty() => batches[0].end_block as f64,
                    _ => batch_number,
                }
            }
            JobType::SnosRun => {
                let batch_number = parse_string(&internal_id)?;

                // Fetch the batch from the database
                match config.database().get_snos_batches_by_indices(vec![batch_number as u64]).await {
                    Ok(batches) if !batches.is_empty() => batches[0].end_block as f64,
                    _ => batch_number,
                }
            }
            _ => parse_string(&internal_id)?,
        };

        ORCHESTRATOR_METRICS.block_gauge.record(block_number, &attributes);
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
        Ok(())
    }

    /// Processes the job, increments the process attempt count, and updates the status of the job in the
    /// DB. It then adds the job to the verification queue.
    ///
    /// # Arguments
    /// * `id` - UUID of the job to process
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # State Transitions
    /// * `Created` -> `LockedForProcessing` -> `PendingVerification`
    /// * `VerificationFailed` -> `LockedForProcessing` -> `PendingVerification`
    /// * `PendingRetry` -> `LockedForProcessing` -> `PendingVerification`
    ///
    /// # Metrics
    /// * Updates block gauge
    /// * Records successful job operations
    /// * Tracks job response time
    ///
    /// # Notes
    /// * Only processes jobs in Created, VerificationFailed, or PendingRetry status
    /// * Updates the job version to prevent concurrent processing
    /// * Adds processing completion timestamp to metadata
    /// * Automatically adds the job to verification queue upon successful processing
    pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = job.internal_id.clone();
        info!(
            log_type = "starting",
            category = "general",
            function_type = "process_job",
            block_no = %internal_id,
            "General process job started for block"
        );

        // Calculate and record queue wait time
        let queue_wait_time = Utc::now().signed_duration_since(job.created_at).num_seconds() as f64;
        MetricsRecorder::record_job_processing_started(&job, queue_wait_time);

        debug!(job_id = ?id, status = ?job.status, "Current job status");
        match job.status {
            // We only want to process jobs that are in the created or verification failed state,
            // or if it's been called from the retry endpoint (in this case it would be
            // PendingRetry status) verification failed state means that the previous processing
            // failed, and we want to retry
            JobStatus::Created | JobStatus::VerificationFailed | JobStatus::PendingRetry => {
                info!(job_id = ?id, status = ?job.status, "Processing job");

                // Record retry if this is not the first attempt
                if job.status == JobStatus::VerificationFailed || job.status == JobStatus::PendingRetry {
                    MetricsRecorder::record_job_retry(&job, &job.status.to_string());
                }
            }
            _ => {
                warn!(
                    job_id = ?id,
                    status = ?job.status,
                    job_type = ?job.job_type,
                    internal_id = ?job.internal_id,
                    "Cannot process job with current status"
                );
                return Err(JobError::InvalidStatus { id, job_status: job.status });
            }
        }

        let job_handler = factory::get_job_handler(&job.job_type).await;

        // This updates the version of the job.
        // This ensures that if another thread was about to process the same job,
        // it would fail to update the job in the database because the version would be outdated
        debug!(job_id = ?id, "Updating job status to LockedForProcessing");
        job.metadata.common.process_started_at = Some(Utc::now());

        // Record state transition
        ORCHESTRATOR_METRICS.job_state_transitions.add(
            1.0,
            &[
                KeyValue::new("from_state", job.status.to_string()),
                KeyValue::new("to_state", JobStatus::LockedForProcessing.to_string()),
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            ],
        );
        let mut job = config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::LockedForProcessing)
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await
            .inspect_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to update job status");
            })?;

        // Update job status tracking metrics for LockedForProcessing
        let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job.job_type,
            &JobStatus::LockedForProcessing,
            &job.id.to_string(),
        );

        debug!(job_id = ?id, job_type = ?job.job_type, "Getting job handler");
        let external_id = match AssertUnwindSafe(job_handler.process_job(config.clone(), &mut job)).catch_unwind().await
        {
            Ok(Ok(external_id)) => {
                debug!(job_id = ?id, job_type = ?job.job_type, "Successfully processed job with external ID: {:?}", external_id);
                Span::current().record("external_id", format!("{:?}", external_id).as_str());
                // Add the time of processing to the metadata.
                job.metadata.common.process_completed_at = Some(Utc::now());

                external_id
            }
            Ok(Err(e)) => {
                // TODO: I think most of the times the errors will not be fixed automatically
                // if we just retry. But for some failures like DB issues, it might be possible
                // that retrying will work. So we can add a retry logic here to improve robustness.
                error!(job_id = ?id, error = ?e, "Failed to process job");
                return JobService::move_job_to_failed(&job, config.clone(), format!("Processing failed: {}", e)).await;
            }
            Err(panic) => {
                let panic_msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("Unknown panic message");

                error!(job_id = ?id, panic_msg = %panic_msg, "Job handler panicked during processing");
                return JobService::move_job_to_failed(
                    &job,
                    config.clone(),
                    format!("Job handler panicked with message: {}", panic_msg),
                )
                .await;
            }
        };

        // Increment process attempt counter
        job.metadata.common.process_attempt_no += 1;

        // Update job status and metadata
        debug!(job_id = ?id, "Updating job status to PendingVerification");
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingVerification)
                    .update_metadata(job.metadata.clone())
                    .update_external_id(external_id.clone().into())
                    .build(),
            )
            .await
            .map_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to update job status");
                JobError::from(e)
            })?;

        // Update job status tracking metrics for PendingVerification
        let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job.job_type,
            &JobStatus::PendingVerification,
            &job.id.to_string(),
        );

        // Add to the verification queue
        debug!(job_id = ?id, "Adding job to verification queue");
        JobService::add_job_to_verify_queue(
            config.clone(),
            job.id,
            &job.job_type,
            Some(Duration::from_secs(job_handler.verification_polling_delay_seconds())),
        )
        .await
        .map_err(|e| {
            error!(job_id = ?id, error = ?e, "Failed to add job to verification queue");
            e
        })?;

        let attributes = vec![
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "process_job"),
        ];

        info!(
            log_type = "completed",
            category = "general",
            function_type = "process_job",
            block_no = %internal_id,
            "General process job completed for block"
        );

        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
        Self::register_block_gauge(job.job_type, &job.internal_id, external_id.into(), &attributes)?;

        Ok(())
    }

    /// Verify Job Function
    ///
    /// Verifies the job and updates the status of the job in the DB. If the verification fails, it
    /// retries processing the job if the max attempts have not been exceeded. If the max attempts have
    /// been exceeded, it marks the job as timed out. If the verification is still pending, it pushes
    /// the job back to the queue.
    ///
    /// # Arguments
    /// * `id` - UUID of the job to verify
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # State Transitions
    /// * `PendingVerification` -> `Completed` (on successful verification)
    /// * `PendingVerification` -> `VerificationFailed` (on verification rejection)
    /// * `PendingVerification` -> `VerificationTimeout` (max attempts reached)
    ///
    /// # Metrics
    /// * Records verification time if the processing completion timestamp exists
    /// * Updates block gauge and job operation metrics
    /// * Tracks successful operations and response time
    ///
    /// # Notes
    /// * Only jobs in `PendingVerification` or `VerificationTimeout` status can be verified
    /// * Automatically retries processing if verification fails and the max attempts are not reached
    /// * Removes processing_finished_at from metadata upon successful verification
    pub async fn verify_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = job.internal_id.clone();
        if !matches!(job.external_id, ExternalId::Number(0)) {
            Span::current().record("external_id", format!("{:?}", job.external_id).as_str());
        }
        info!(log_type = "starting", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job started for block");

        match job.status {
            // Jobs with `VerificationTimeout` will be retired manually after resetting verification attempt number to 0.
            JobStatus::PendingVerification | JobStatus::VerificationTimeout => {
                info!(job_id = ?id, status = ?job.status, "Proceeding with verification");
            }
            _ => {
                error!(job_id = ?id, status = ?job.status, "Invalid job status for verification");
                return Err(JobError::InvalidStatus { id, job_status: job.status });
            }
        }

        let job_handler = factory::get_job_handler(&job.job_type).await;
        debug!(job_id = ?id, "Verifying job with handler");

        job.metadata.common.verification_started_at = Some(Utc::now());

        // Record verification started
        MetricsRecorder::record_verification_started(&job);

        let mut job = config
            .database()
            .update_job(&job, JobItemUpdates::new().update_metadata(job.metadata.clone()).build())
            .await
            .map_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to update job status");
                e
            })?;

        let verification_status = job_handler.verify_job(config.clone(), &mut job).await?;
        Span::current().record("verification_status", format!("{:?}", &verification_status));

        let mut attributes = vec![
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "verify_job"),
            KeyValue::new("operation_verification_status", format!("{:?}", &verification_status)),
        ];
        let mut operation_job_status: Option<JobStatus> = None;

        match verification_status {
            JobVerificationStatus::Verified => {
                info!(job_id = ?id, "Job verified successfully");
                // Calculate verification time if processing completion timestamp exists
                if let Some(verification_time) = job.metadata.common.verification_started_at {
                    let time_taken = (Utc::now() - verification_time).num_milliseconds();
                    ORCHESTRATOR_METRICS.verification_time.record(
                        time_taken as f64,
                        &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))],
                    );
                } else {
                    warn!("Failed to calculate verification time: Missing processing completion timestamp");
                }

                // Update verification completed timestamp and update status
                job.metadata.common.verification_completed_at = Some(Utc::now());

                // Record E2E latency and completion
                let e2e_duration = Utc::now().signed_duration_since(job.created_at).num_seconds() as f64;
                MetricsRecorder::record_job_completed(&job, e2e_duration);

                // Check SLA compliance (example: 5 minute SLA)
                MetricsRecorder::check_and_record_sla_breach(&job, 300, "e2e_time");

                config
                    .database()
                    .update_job(
                        &job,
                        JobItemUpdates::new()
                            .update_metadata(job.metadata.clone())
                            .update_status(JobStatus::Completed)
                            .build(),
                    )
                    .await
                    .map_err(|e| {
                        error!(job_id = ?id, error = ?e, "Failed to update job status to Completed");
                        e
                    })?;

                // Update job status tracking metrics for Completed
                let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
                ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
                    block_num,
                    &job.job_type,
                    &JobStatus::Completed,
                    &job.id.to_string(),
                );

                operation_job_status = Some(JobStatus::Completed);
            }
            JobVerificationStatus::Rejected(e) => {
                error!(job_id = ?id, error = ?e, "Job verification rejected");

                // Update metadata with error information
                job.metadata.common.failure_reason = Some(e.clone());
                operation_job_status = Some(JobStatus::VerificationFailed);

                // Record job failure
                MetricsRecorder::record_job_failed(&job, &e);

                if job.metadata.common.process_attempt_no < job_handler.max_process_attempts() {
                    info!(
                        job_id = ?id,
                        attempt = job.metadata.common.process_attempt_no + 1,
                        "Verification failed. Retrying job processing"
                    );

                    config
                        .database()
                        .update_job(
                            &job,
                            JobItemUpdates::new()
                                .update_status(JobStatus::VerificationFailed)
                                .update_metadata(job.metadata.clone())
                                .build(),
                        )
                        .await
                        .map_err(|e| {
                            error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationFailed");
                            e
                        })?;
                    JobService::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await?;
                } else {
                    warn!(job_id = ?id, "Max process attempts reached. Job will not be retried");

                    // Record job abandoned after max retries
                    let retry_count = job.metadata.common.process_attempt_no;
                    MetricsRecorder::record_job_abandoned(&job, retry_count as i32);
                    return JobService::move_job_to_failed(
                        &job,
                        config.clone(),
                        format!(
                            "Verification rejected. Max process attempts reached: {}",
                            job.metadata.common.process_attempt_no
                        ),
                    )
                    .await;
                }
            }
            JobVerificationStatus::Pending => {
                debug!(job_id = ?id, "Job verification still pending");

                if job.metadata.common.verification_attempt_no >= job_handler.max_verification_attempts() {
                    warn!(job_id = ?id, "Max verification attempts reached. Marking job as timed out");

                    // Record timeout metric
                    MetricsRecorder::record_job_timeout(&job);

                    config
                        .database()
                        .update_job(&job, JobItemUpdates::new().update_status(JobStatus::VerificationTimeout).build())
                        .await
                        .map_err(|e| {
                            error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationTimeout");
                            JobError::from(e)
                        })?;
                    operation_job_status = Some(JobStatus::VerificationTimeout);
                } else {
                    // Increment verification attempts
                    job.metadata.common.verification_attempt_no += 1;

                    config
                        .database()
                        .update_job(&job, JobItemUpdates::new().update_metadata(job.metadata.clone()).build())
                        .await
                        .map_err(|e| {
                            error!(job_id = ?id, error = ?e, "Failed to update job metadata");
                            JobError::from(e)
                        })?;

                    debug!(job_id = ?id, "Adding job back to verification queue");
                    JobService::add_job_to_verify_queue(
                        config.clone(),
                        job.id,
                        &job.job_type,
                        Some(Duration::from_secs(job_handler.verification_polling_delay_seconds())),
                    )
                    .await
                    .map_err(|e| {
                        error!(job_id = ?id, error = ?e, "Failed to add job to verification queue");
                        e
                    })?;
                }
            }
        };

        if let Some(job_status) = operation_job_status {
            attributes.push(KeyValue::new("operation_job_status", format!("{}", job_status)));
        }

        info!(log_type = "completed", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job completed for block");
        let duration = start.elapsed();
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);
        Self::register_block_gauge(job.job_type, &job.internal_id, job.external_id, &attributes)?;
        Ok(())
    }

    /// Terminates the job and updates the status of the job in the DB.
    ///
    /// # Arguments
    /// * `id` - UUID of the job to handle failure for
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # Notes
    /// * Logs error if the job status `Completed` is existing on DL queue
    /// * Updates job status to Failed and records failure reason in metadata
    /// * Updates metrics for failed jobs
    pub async fn handle_job_failure(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let job = JobService::get_job(id, config.clone()).await?.clone();
        let internal_id = job.internal_id.clone();
        info!(log_type = "starting", category = "general", function_type = "handle_job_failure", block_no = %internal_id, "General handle job failure started for block");

        debug!(job_id = ?id, job_status = ?job.status, job_type = ?job.job_type, block_no = %internal_id, "Job details for failure handling for block");
        let status = job.status.clone().to_string();
        JobService::move_job_to_failed(
            &job,
            config.clone(),
            format!("Received failure queue message for job with status: {}", status),
        )
        .await
    }

    /// Retries a failed job by reprocessing it.
    /// Only jobs with Failed status can be retried.
    ///
    /// # Arguments
    /// * `id` - UUID of the job to retry
    /// * `config` - Shared configuration
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Success or an error
    ///
    /// # State Transitions
    /// * `Failed` -> `PendingRetry` -> (normal processing flow)
    ///
    /// # Notes
    /// * Only jobs in Failed status can be retried
    /// * Transitions through PendingRetry status before normal processing
    /// * Uses standard process_job function after status update
    pub async fn retry_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = job.internal_id.clone();

        info!(
            log_type = "starting",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "General retry job started for block"
        );
        if job.status != JobStatus::Failed {
            error!(
                job_id = ?id,
                status = ?job.status,
                "Cannot retry job: invalid status"
            );
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }

        // Increment the retry counter in common metadata
        job.metadata.common.process_retry_attempt_no += 1;
        // Reset the process attempt counter to 0, to ensure a fresh start
        job.metadata.common.process_attempt_no = 0;

        debug!(
            job_id = ?id,
            retry_count = job.metadata.common.process_retry_attempt_no,
            "Incrementing process retry attempt counter"
        );

        // Update job status and metadata to PendingRetry before processing
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingRetry)
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await
            .map_err(|e| {
                error!(
                    job_id = ?id,
                    error = ?e,
                    "Failed to update job status to PendingRetry"
                );
                e
            })?;

        JobService::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await.map_err(|e| {
            error!(
                log_type = "error",
                category = "general",
                function_type = "retry_job",
                block_no = %internal_id,
                error = %e,
                "Failed to add job to process queue"
            );
            e
        })?;

        info!(
            log_type = "completed",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "Successfully queued job for retry"
        );

        Ok(())
    }

    fn register_block_gauge(
        job_type: JobType,
        internal_id: &str,
        external_id: ExternalId,
        attributes: &[KeyValue],
    ) -> Result<(), JobError> {
        let block_number = if let JobType::StateTransition = job_type {
            parse_string(
                external_id
                    .unwrap_string()
                    .map_err(|e| JobError::Other(OtherError::from(format!("Could not parse string: {e}"))))?,
            )
        } else {
            parse_string(internal_id)
        }?;

        ORCHESTRATOR_METRICS.block_gauge.record(block_number, attributes);
        Ok(())
    }
    /// To get Box<dyn Worker> handler from `WorkerTriggerType`.
    pub fn get_worker_handler_from_worker_trigger_type(worker_trigger_type: WorkerTriggerType) -> Box<dyn JobTrigger> {
        match worker_trigger_type {
            WorkerTriggerType::Snos => Box::new(SnosJobTrigger),
            WorkerTriggerType::Proving => Box::new(ProvingJobTrigger),
            WorkerTriggerType::DataSubmission => Box::new(DataSubmissionJobTrigger),
            WorkerTriggerType::ProofRegistration => Box::new(ProofRegistrationJobTrigger),
            WorkerTriggerType::UpdateState => Box::new(UpdateStateJobTrigger),
            WorkerTriggerType::Batching => Box::new(BatchingTrigger),
            WorkerTriggerType::Aggregator => Box::new(AggregatorJobTrigger),
        }
    }
}
