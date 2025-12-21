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
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::jobs::WorkerTriggerType;

use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::utils::metrics_recorder::MetricsRecorder;
#[double]
use crate::worker::event_handler::factory::factory;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::event_handler::triggers::aggregator::AggregatorJobTrigger;
use crate::worker::event_handler::triggers::aggregator_batching::AggregatorBatchingTrigger;
use crate::worker::event_handler::triggers::data_submission_worker::DataSubmissionJobTrigger;
use crate::worker::event_handler::triggers::proof_registration::ProofRegistrationJobTrigger;
use crate::worker::event_handler::triggers::proving::ProvingJobTrigger;
use crate::worker::event_handler::triggers::snos::SnosJobTrigger;
use crate::worker::event_handler::triggers::snos_batching::SnosBatchingTrigger;
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
        debug!(
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

        // Note: No need to queue for processing - workers poll MongoDB directly
        // Jobs with status Created are automatically picked up

        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("operation_type", "create_job"),
        ];

        info!(job_id = %job_item.id, "Successfully created new {:?} job with internal id {}", job_type, internal_id);

        debug!(
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
    /// * `Created` -> `LockedForProcessing` -> `Processed`
    /// * `PendingRetryProcessing` -> `LockedForProcessing` -> `Processed`
    /// * On failure with retries left: -> `PendingRetryProcessing`
    /// * On failure without retries: -> `ProcessingFailed`
    ///
    /// # Metrics
    /// * Updates block gauge
    /// * Records successful job operations
    /// * Tracks job response time
    ///
    /// # Notes
    /// * Only processes jobs in Created or PendingRetryProcessing status
    /// * Updates the job version to prevent concurrent processing
    /// * Adds processing completion timestamp to metadata
    /// * Automatically adds the job to verification queue upon successful processing
    pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = &job.internal_id;
        // Generate a unique orchestrator ID for this processing attempt
        // This is used for orphan detection and concurrency control
        let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());
        debug!(
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

        // Status validation: only accept Created or PendingRetryProcessing
        match job.status {
            JobStatus::Created => {
                debug!(job_id = ?id, "Processing new job");
            }
            JobStatus::PendingRetryProcessing => {
                info!(
                    job_id = ?id,
                    attempt = job.metadata.common.process_attempt_no + 1,
                    "Processing retry for job"
                );
                MetricsRecorder::record_job_retry(&job, &job.status.to_string());
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

        // Check if dependencies are ready before processing
        // In worker mode, if dependencies aren't ready, we just skip and let the next poll pick it up
        if let Err(retry_delay) = job_handler.check_ready_to_process(config.clone()).await {
            debug!(job_id = ?id, job_type = ?job.job_type, delay_secs = ?retry_delay.as_secs(), "Dependencies not ready, skipping (worker mode will retry)");
            return Ok(());
        }

        // Prepare metadata updates for atomic claim
        job.metadata.common.process_started_at = Some(Utc::now());
        job.metadata.common.process_attempt_no += 1;

        // Record state transition
        ORCHESTRATOR_METRICS.job_state_transitions.add(
            1.0,
            &[
                KeyValue::new("from_state", job.status.to_string()),
                KeyValue::new("to_state", JobStatus::LockedForProcessing.to_string()),
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            ],
        );

        // Atomically claim the job: set status to LockedForProcessing + set claimed_by + update metadata
        // This uses optimistic locking (version check) to prevent race conditions
        // If another worker tries to claim the same job, this update will fail due to version mismatch
        // claimed_by is essential for orphan detection - if orchestrator dies while processing,
        // heal_orphaned_jobs() will detect it after timeout and reset to Created
        let mut job = config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::LockedForProcessing)
                    .update_claimed_by(Some(orchestrator_id.clone()))
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await
            .inspect_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to claim job (likely another worker claimed it)");
            })?;

        info!(
            job_id = ?id,
            job_type = ?job.job_type,
            orchestrator_id = %orchestrator_id,
            from_status = "Created/PendingRetry",
            "Atomically claimed job for processing"
        );

        // Update job status tracking metrics for LockedForProcessing
        let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job.job_type,
            &JobStatus::LockedForProcessing,
            &job.id.to_string(),
        );

        let external_id = match AssertUnwindSafe(job_handler.process_job(config.clone(), &mut job)).catch_unwind().await
        {
            Ok(Ok(external_id)) => {
                Span::current().record("external_id", format!("{:?}", external_id).as_str());
                // Add the time of processing to the metadata.
                job.metadata.common.process_completed_at = Some(Utc::now());

                external_id
            }
            Ok(Err(e)) => {
                error!(
                    job_id = ?id,
                    job_type = ?job.job_type,
                    internal_id = %job.internal_id,
                    status = ?job.status,
                    error = ?e,
                    "Failed to process job"
                );

                return Self::handle_processing_failure(
                    &job,
                    &job_handler,
                    config.clone(),
                    format!("Processing failed: {}", e),
                )
                .await;
            }
            Err(panic) => {
                let panic_msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("Unknown panic message");

                error!(job_id = ?id, panic_msg = %panic_msg, "Job handler panicked during processing");

                return Self::handle_processing_failure(
                    &job,
                    &job_handler,
                    config.clone(),
                    format!("Panic: {}", panic_msg),
                )
                .await;
            }
        };

        // MULTI-ORCHESTRATOR FIX: Clear claim when moving to Processed
        // This allows any orchestrator to claim it for verification
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::Processed)
                    .update_metadata(job.metadata.clone())
                    .update_external_id(external_id.clone().into())
                    .clear_claim() // Clear worker mode claim
                    .build(),
            )
            .await
            .map_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to update job status");
                JobError::from(e)
            })?;

        info!(
            job_id = ?id,
            job_type = ?job.job_type,
            external_id = ?external_id,
            from_status = ?JobStatus::LockedForProcessing,
            "Updating status of {:?} job {} to {}", job.job_type, job.internal_id, JobStatus::Processed
        );

        // Update job status tracking metrics for Processed
        let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job.job_type,
            &JobStatus::Processed,
            &job.id.to_string(),
        );

        // Add to the verification queue
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

        debug!(
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
    /// * `Processed` -> `LockedForVerification` -> `Completed` (on successful verification)
    /// * `PendingRetryVerification` -> `LockedForVerification` -> `Completed` (on retry success)
    /// * On VerificationStatus::Rejected: -> `VerificationFailed` (terminal)
    /// * On VerificationStatus::Pending with retries left: -> `Processed` (requeue)
    /// * On VerificationStatus::Pending max retries: -> `PendingRetryVerification`
    ///
    /// # Metrics
    /// * Records verification time if the processing completion timestamp exists
    /// * Updates block gauge and job operation metrics
    /// * Tracks successful operations and response time
    ///
    /// # Notes
    /// * Only jobs in `Processed` or `PendingRetryVerification` status can be verified
    /// * Atomically claims job by setting `LockedForVerification` status
    /// * Clears claimed_by on completion, rejection, or when moving back to Processed
    pub async fn verify_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = &job.internal_id;
        if !matches!(job.external_id, ExternalId::Number(0)) {
            Span::current().record("external_id", format!("{:?}", job.external_id).as_str());
        }
        debug!(log_type = "starting", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job started for block");

        // Generate a unique orchestrator ID for this verification attempt
        let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());

        // Status validation: only accept Processed or PendingRetryVerification
        match job.status {
            JobStatus::Processed => {
                debug!(job_id = ?id, "Verifying processed job");
            }
            JobStatus::PendingRetryVerification => {
                info!(
                    job_id = ?id,
                    attempt = job.metadata.common.verification_attempt_no + 1,
                    "Verifying retry for job"
                );
            }
            _ => {
                error!(job_id = ?id, status = ?job.status, "Invalid job status for verification");
                return Err(JobError::InvalidStatus { id, job_status: job.status });
            }
        }

        let job_handler = factory::get_job_handler(&job.job_type).await;

        // Prepare metadata updates for atomic claim
        job.metadata.common.verification_started_at = Some(Utc::now());
        job.metadata.common.verification_attempt_no += 1;

        // Record verification started
        MetricsRecorder::record_verification_started(&job);

        // Record state transition
        ORCHESTRATOR_METRICS.job_state_transitions.add(
            1.0,
            &[
                KeyValue::new("from_state", job.status.to_string()),
                KeyValue::new("to_state", JobStatus::LockedForVerification.to_string()),
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            ],
        );

        // Atomically claim the job for verification
        let mut job = config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::LockedForVerification)
                    .update_claimed_by(Some(orchestrator_id.clone()))
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await
            .inspect_err(|e| {
                error!(job_id = ?id, error = ?e, "Failed to claim job for verification (likely another worker claimed it)");
            })?;

        info!(
            job_id = ?id,
            job_type = ?job.job_type,
            orchestrator_id = %orchestrator_id,
            from_status = "Processed/PendingRetryVerification",
            "Atomically claimed job for verification"
        );

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
                // Calculate verification time if verification started timestamp exists
                if let Some(verification_time) = job.metadata.common.verification_started_at {
                    let time_taken = (Utc::now() - verification_time).num_milliseconds();
                    ORCHESTRATOR_METRICS.verification_time.record(
                        time_taken as f64,
                        &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))],
                    );
                } else {
                    warn!("Failed to calculate verification time: Missing verification started timestamp");
                }

                // Update verification completed timestamp
                job.metadata.common.verification_completed_at = Some(Utc::now());

                // Record E2E latency and completion
                let e2e_duration = Utc::now().signed_duration_since(job.created_at).num_seconds() as f64;
                MetricsRecorder::record_job_completed(&job, e2e_duration);

                // Check SLA compliance (example: 5 minute SLA)
                MetricsRecorder::check_and_record_sla_breach(&job, 300, "e2e_time");

                // Update to Completed and clear claim
                config
                    .database()
                    .update_job(
                        &job,
                        JobItemUpdates::new()
                            .update_status(JobStatus::Completed)
                            .update_metadata(job.metadata.clone())
                            .clear_claim()
                            .build(),
                    )
                    .await
                    .map_err(|e| {
                        error!(job_id = ?id, error = ?e, "Failed to update job status to Completed");
                        e
                    })?;

                info!(
                    job_id = ?id,
                    job_type = ?job.job_type,
                    external_id = ?job.external_id,
                    from_status = ?JobStatus::LockedForVerification,
                    "Updating status of {:?} job {} to {}", job.job_type, job.internal_id, JobStatus::Completed
                );

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
                error!(job_id = ?id, error = ?e, "Job verification rejected - moving to terminal VerificationFailed state");

                // Update metadata with error information
                job.metadata.common.failure_reason = Some(e.clone());
                operation_job_status = Some(JobStatus::VerificationFailed);

                // Record job failure
                MetricsRecorder::record_job_failed(&job, &e);

                // Verification rejection is terminal - move to VerificationFailed
                config
                    .database()
                    .update_job(
                        &job,
                        JobItemUpdates::new()
                            .update_status(JobStatus::VerificationFailed)
                            .update_metadata(job.metadata.clone())
                            .clear_claim()
                            .build(),
                    )
                    .await
                    .map_err(|e| {
                        error!(job_id = ?id, error = ?e, "Failed to update job status to VerificationFailed");
                        e
                    })?;

                info!(
                    job_id = ?id,
                    job_type = ?job.job_type,
                    "Job moved to VerificationFailed (terminal state) after verification rejection"
                );
            }
            JobVerificationStatus::Pending => {
                if job.metadata.common.verification_attempt_no >= job_handler.max_verification_attempts() {
                    warn!(job_id = ?id, "Max verification attempts reached. Moving to PendingRetryVerification");

                    // Record timeout metric
                    MetricsRecorder::record_job_timeout(&job);

                    // Move to PendingRetryVerification and clear claim
                    config
                        .database()
                        .update_job(
                            &job,
                            JobItemUpdates::new()
                                .update_status(JobStatus::PendingRetryVerification)
                                .update_metadata(job.metadata.clone())
                                .clear_claim()
                                .build(),
                        )
                        .await
                        .map_err(|e| {
                            error!(job_id = ?id, error = ?e, "Failed to update job status to PendingRetryVerification");
                            JobError::from(e)
                        })?;
                    operation_job_status = Some(JobStatus::PendingRetryVerification);
                } else {
                    // Still pending - move back to Processed and clear claim for retry
                    debug!(
                        job_id = ?id,
                        attempt = job.metadata.common.verification_attempt_no,
                        "Verification still pending, moving back to Processed for retry"
                    );

                    // Calculate available_at time (60 seconds from now)
                    let available_at = Utc::now() + chrono::Duration::seconds(60);

                    config
                        .database()
                        .update_job(
                            &job,
                            JobItemUpdates::new()
                                .update_status(JobStatus::Processed)
                                .update_metadata(job.metadata.clone())
                                .clear_claim()
                                .build(),
                        )
                        .await
                        .map_err(|e| {
                            error!(job_id = ?id, error = ?e, "Failed to update job back to Processed");
                            JobError::from(e)
                        })?;

                    debug!(job_id = ?id, available_at = %available_at, "Adding job back to verification queue");
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

        debug!(log_type = "completed", category = "general", function_type = "verify_job", block_no = %internal_id, "General verify job completed for block");
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
        let internal_id = &job.internal_id;
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
    /// * `ProcessingFailed` -> `PendingRetryProcessing` -> (normal processing flow)
    ///
    /// # Notes
    /// * Only jobs in ProcessingFailed status can be retried
    /// * Transitions through PendingRetryProcessing status before normal processing
    /// * Uses standard process_job function after status update
    pub async fn retry_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = JobService::get_job(id, config.clone()).await?;
        let internal_id = &job.internal_id;

        info!(
            log_type = "starting",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "General retry job started for block"
        );
        if job.status != JobStatus::ProcessingFailed {
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

        // Update job status and metadata to PendingRetryProcessing before processing
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingRetryProcessing)
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await
            .map_err(|e| {
                error!(
                    job_id = ?id,
                    error = ?e,
                    "Failed to update job status to PendingRetryProcessing"
                );
                e
            })?;

        // Note: No need to queue - workers poll MongoDB directly for PendingRetryProcessing jobs

        info!(
            log_type = "completed",
            category = "general",
            function_type = "retry_job",
            block_no = %internal_id,
            "Successfully marked job for retry"
        );

        Ok(())
    }

    /// Helper function to handle processing failures.
    /// Determines whether to retry or mark as permanently failed based on attempt count.
    ///
    /// # Arguments
    /// * `job` - The job that failed
    /// * `job_handler` - Handler for this job type (provides max_process_attempts)
    /// * `config` - Shared configuration
    /// * `failure_reason` - Description of why processing failed
    ///
    /// # Returns
    /// * `Result<(), JobError>` - Always returns Ok after handling the failure
    ///
    /// # State Transitions
    /// * If retries left: -> `PendingRetryProcessing`
    /// * If max retries exhausted: -> `ProcessingFailed` (terminal)
    async fn handle_processing_failure(
        job: &JobItem,
        job_handler: &Box<dyn JobHandlerTrait>,
        config: Arc<Config>,
        failure_reason: String,
    ) -> Result<(), JobError> {
        let mut job = job.clone();
        job.metadata.common.failure_reason = Some(failure_reason.clone());

        if job.metadata.common.process_attempt_no < job_handler.max_process_attempts() {
            // Still have retries left - move to PendingRetryProcessing
            info!(
                job_id = ?job.id,
                attempt = job.metadata.common.process_attempt_no,
                max_attempts = job_handler.max_process_attempts(),
                "Processing failed. Retrying job"
            );

            config
                .database()
                .update_job(
                    &job,
                    JobItemUpdates::new()
                        .update_status(JobStatus::PendingRetryProcessing)
                        .update_metadata(job.metadata.clone())
                        .clear_claim()
                        .build(),
                )
                .await
                .map_err(|e| {
                    error!(job_id = ?job.id, error = ?e, "Failed to update job to PendingRetryProcessing");
                    JobError::from(e)
                })?;

            // Note: No need to queue - workers poll MongoDB directly for PendingRetryProcessing jobs
        } else {
            // Max retries reached - move to ProcessingFailed (terminal state)
            warn!(
                job_id = ?job.id,
                attempts = job.metadata.common.process_attempt_no,
                "Max process attempts reached. Moving to ProcessingFailed (terminal state)"
            );

            MetricsRecorder::record_job_abandoned(&job, job.metadata.common.process_attempt_no as i32);

            config
                .database()
                .update_job(
                    &job,
                    JobItemUpdates::new()
                        .update_status(JobStatus::ProcessingFailed)
                        .update_metadata(job.metadata.clone())
                        .clear_claim()
                        .build(),
                )
                .await
                .map_err(|e| {
                    error!(job_id = ?job.id, error = ?e, "Failed to update job to ProcessingFailed");
                    JobError::from(e)
                })?;

            info!(
                job_id = ?job.id,
                job_type = ?job.job_type,
                "Job moved to ProcessingFailed after {} attempts",
                job.metadata.common.process_attempt_no
            );
        }

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
            WorkerTriggerType::AggregatorBatching => Box::new(AggregatorBatchingTrigger),
            WorkerTriggerType::SnosBatching => Box::new(SnosBatchingTrigger),
            WorkerTriggerType::Snos => Box::new(SnosJobTrigger),
            WorkerTriggerType::Proving => Box::new(ProvingJobTrigger),
            WorkerTriggerType::DataSubmission => Box::new(DataSubmissionJobTrigger),
            WorkerTriggerType::ProofRegistration => Box::new(ProofRegistrationJobTrigger),
            WorkerTriggerType::Aggregator => Box::new(AggregatorJobTrigger),
            WorkerTriggerType::UpdateState => Box::new(UpdateStateJobTrigger),
        }
    }
}
