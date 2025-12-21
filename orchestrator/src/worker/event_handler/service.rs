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
    /// Creates a new job in the database. Skips if job with same internal_id exists.
    pub async fn create_job(
        job_type: JobType,
        internal_id: String,
        mut metadata: JobMetadata,
        config: Arc<Config>,
    ) -> Result<(), JobError> {
        let start = Instant::now();

        // Skip if job already exists
        if config.database().get_job_by_internal_id_and_type(&internal_id, &job_type).await?.is_some() {
            warn!("Job already exists: {:?} {}", job_type, internal_id);
            return Ok(());
        }

        metadata.common.orchestrator_version = Some(crate::types::constant::ORCHESTRATOR_VERSION.to_string());

        let job_handler = factory::get_job_handler(&job_type).await;
        let job_item = job_handler.create_job(internal_id.clone(), metadata).await?;
        config.database().create_job(job_item.clone()).await?;

        MetricsRecorder::record_job_created(&job_item);

        let block_num = parse_string(&internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job_type,
            &JobStatus::Created,
            &job_item.id.to_string(),
        );

        info!("Created {:?} job {} (id: {})", job_type, internal_id, job_item.id);

        // Record metrics
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("operation_type", "create_job"),
        ];
        let block_number = Self::get_block_number_for_metrics(&job_type, &internal_id, &config).await;
        ORCHESTRATOR_METRICS.block_gauge.record(block_number, &attributes);
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(start.elapsed().as_secs_f64(), &attributes);

        Ok(())
    }

    /// Processes a job: Created/PendingRetryProcessing -> LockedForProcessing -> Processed
    pub async fn process_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());

        // Validate status
        match job.status {
            JobStatus::Created => {}
            JobStatus::PendingRetryProcessing => {
                info!("Retrying job {} (attempt {})", job.internal_id, job.metadata.common.process_attempt_no + 1);
                MetricsRecorder::record_job_retry(&job, &job.status.to_string());
            }
            _ => {
                warn!("Cannot process job {} with status {:?}", job.internal_id, job.status);
                return Err(JobError::InvalidStatus { id, job_status: job.status });
            }
        }

        let job_handler = factory::get_job_handler(&job.job_type).await;

        // Check dependencies - skip if not ready (worker will retry later)
        if let Err(retry_delay) = job_handler.check_ready_to_process(config.clone()).await {
            debug!(job_id = %id, delay_secs = retry_delay.as_secs(), "Dependencies not ready, skipping");
            return Ok(());
        }

        // Record queue wait time
        let queue_wait_time = Utc::now().signed_duration_since(job.created_at).num_seconds() as f64;
        MetricsRecorder::record_job_processing_started(&job, queue_wait_time);

        // Prepare metadata and claim job atomically
        job.metadata.common.process_started_at = Some(Utc::now());
        job.metadata.common.process_attempt_no += 1;

        Self::record_state_transition(&job, JobStatus::LockedForProcessing);

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
            .await?;

        info!("Processing job {} ({:?})", job.internal_id, job.job_type);

        Self::update_job_status_tracker(&job, JobStatus::LockedForProcessing);

        // Process the job
        let external_id = match AssertUnwindSafe(job_handler.process_job(config.clone(), &mut job)).catch_unwind().await
        {
            Ok(Ok(external_id)) => {
                Span::current().record("external_id", format!("{:?}", external_id).as_str());
                job.metadata.common.process_completed_at = Some(Utc::now());
                external_id
            }
            Ok(Err(e)) => {
                error!("Failed to process job {}: {}", job.internal_id, e);
                return Self::handle_processing_failure(
                    &job,
                    &job_handler,
                    config,
                    format!("Processing failed: {}", e),
                )
                .await;
            }
            Err(panic) => {
                let msg = Self::extract_panic_message(&panic);
                error!("Job {} panicked: {}", job.internal_id, msg);
                return Self::handle_processing_failure(&job, &job_handler, config, format!("Panic: {}", msg)).await;
            }
        };

        // Update to Processed and clear claim
        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::Processed)
                    .update_metadata(job.metadata.clone())
                    .update_external_id(external_id.clone().into())
                    .clear_claim()
                    .build(),
            )
            .await?;

        info!("Processed job {} -> verification queue", job.internal_id);

        Self::update_job_status_tracker(&job, JobStatus::Processed);

        Self::record_success_metrics(&job, "process_job", external_id.into(), start.elapsed());
        Ok(())
    }

    /// Verifies a job: Processed/PendingRetryVerification -> LockedForVerification -> Completed/Failed
    pub async fn verify_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let start = Instant::now();
        let mut job = JobService::get_job(id, config.clone()).await?;
        let orchestrator_id = format!("orchestrator-{}", uuid::Uuid::new_v4());

        if !matches!(job.external_id, ExternalId::Number(0)) {
            Span::current().record("external_id", format!("{:?}", job.external_id).as_str());
        }

        // Validate status
        match job.status {
            JobStatus::Processed => {}
            JobStatus::PendingRetryVerification => {
                info!(
                    "Retrying verification for job {} (attempt {})",
                    job.internal_id,
                    job.metadata.common.verification_attempt_no + 1
                );
            }
            _ => {
                error!("Cannot verify job {} with status {:?}", job.internal_id, job.status);
                return Err(JobError::InvalidStatus { id, job_status: job.status });
            }
        }

        let job_handler = factory::get_job_handler(&job.job_type).await;

        // Prepare metadata and claim job atomically
        job.metadata.common.verification_started_at = Some(Utc::now());
        job.metadata.common.verification_attempt_no += 1;

        MetricsRecorder::record_verification_started(&job);
        Self::record_state_transition(&job, JobStatus::LockedForVerification);

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
            .await?;

        info!("Verifying job {} ({:?})", job.internal_id, job.job_type);

        let verification_status = job_handler.verify_job(config.clone(), &mut job).await?;
        Span::current().record("verification_status", format!("{:?}", &verification_status));

        let final_status = match verification_status {
            JobVerificationStatus::Verified => {
                Self::record_verification_time(&job);
                job.metadata.common.verification_completed_at = Some(Utc::now());

                let e2e_duration = Utc::now().signed_duration_since(job.created_at).num_seconds() as f64;
                MetricsRecorder::record_job_completed(&job, e2e_duration);
                MetricsRecorder::check_and_record_sla_breach(&job, 300, "e2e_time");

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
                    .await?;

                info!("Completed job {}", job.internal_id);
                Self::update_job_status_tracker(&job, JobStatus::Completed);
                Some(JobStatus::Completed)
            }

            JobVerificationStatus::Rejected(ref reason) => {
                error!("Job {} verification rejected: {}", job.internal_id, reason);
                job.metadata.common.failure_reason = Some(reason.clone());
                MetricsRecorder::record_job_failed(&job, &reason);

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
                    .await?;

                Some(JobStatus::VerificationFailed)
            }

            JobVerificationStatus::Pending => {
                if job.metadata.common.verification_attempt_no >= job_handler.max_verification_attempts() {
                    warn!("Job {} max verification attempts reached", job.internal_id);
                    MetricsRecorder::record_job_timeout(&job);

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
                        .await?;

                    Some(JobStatus::PendingRetryVerification)
                } else {
                    // Still pending - move back to Processed for retry
                    debug!(
                        "Job {} verification pending, attempt {}",
                        job.internal_id, job.metadata.common.verification_attempt_no
                    );

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
                        .await?;

                    None
                }
            }
        };

        Self::record_verify_metrics(&job, final_status, &verification_status, start.elapsed());
        Ok(())
    }

    /// Handles a job from the failure queue - marks it as ProcessingFailed
    pub async fn handle_job_failure(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let job = JobService::get_job(id, config.clone()).await?;
        info!("Handling failure for job {}", job.internal_id);
        JobService::move_job_to_failed(
            &job,
            config,
            format!("Received failure queue message with status: {}", job.status),
        )
        .await
    }

    /// Retries a ProcessingFailed job by moving it to PendingRetryProcessing
    pub async fn retry_job(id: Uuid, config: Arc<Config>) -> Result<(), JobError> {
        let mut job = JobService::get_job(id, config.clone()).await?;

        if job.status != JobStatus::ProcessingFailed {
            error!("Cannot retry job {} with status {:?}", job.internal_id, job.status);
            return Err(JobError::InvalidStatus { id, job_status: job.status });
        }

        job.metadata.common.process_retry_attempt_no += 1;
        job.metadata.common.process_attempt_no = 0;

        config
            .database()
            .update_job(
                &job,
                JobItemUpdates::new()
                    .update_status(JobStatus::PendingRetryProcessing)
                    .update_metadata(job.metadata.clone())
                    .build(),
            )
            .await?;

        info!("Marked job {} for retry (attempt {})", job.internal_id, job.metadata.common.process_retry_attempt_no);
        Ok(())
    }

    /// Handles processing failure - retries if attempts left, otherwise marks as ProcessingFailed
    async fn handle_processing_failure(
        job: &JobItem,
        job_handler: &Box<dyn JobHandlerTrait>,
        config: Arc<Config>,
        failure_reason: String,
    ) -> Result<(), JobError> {
        let mut job = job.clone();
        job.metadata.common.failure_reason = Some(failure_reason.clone());

        if job.metadata.common.process_attempt_no < job_handler.max_process_attempts() {
            info!(
                "Job {} failed, retrying (attempt {}/{})",
                job.internal_id,
                job.metadata.common.process_attempt_no,
                job_handler.max_process_attempts()
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
                .await?;
        } else {
            warn!(
                "Job {} failed permanently after {} attempts",
                job.internal_id, job.metadata.common.process_attempt_no
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
                .await?;
        }

        Ok(())
    }

    // --- Helper functions ---

    fn extract_panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
        panic
            .downcast_ref::<String>()
            .map(|s| s.as_str())
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("Unknown panic")
            .to_string()
    }

    fn record_state_transition(job: &JobItem, to_status: JobStatus) {
        ORCHESTRATOR_METRICS.job_state_transitions.add(
            1.0,
            &[
                KeyValue::new("from_state", job.status.to_string()),
                KeyValue::new("to_state", to_status.to_string()),
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            ],
        );
    }

    fn update_job_status_tracker(job: &JobItem, status: JobStatus) {
        let block_num = parse_string(&job.internal_id).unwrap_or(0.0) as u64;
        ORCHESTRATOR_METRICS.job_status_tracker.update_job_status(
            block_num,
            &job.job_type,
            &status,
            &job.id.to_string(),
        );
    }

    fn record_verification_time(job: &JobItem) {
        if let Some(started_at) = job.metadata.common.verification_started_at {
            let time_taken = (Utc::now() - started_at).num_milliseconds();
            ORCHESTRATOR_METRICS
                .verification_time
                .record(time_taken as f64, &[KeyValue::new("operation_job_type", format!("{:?}", job.job_type))]);
        }
    }

    async fn get_block_number_for_metrics(job_type: &JobType, internal_id: &str, config: &Arc<Config>) -> f64 {
        let batch_number = parse_string(internal_id).unwrap_or(0.0);
        match job_type {
            JobType::StateTransition | JobType::Aggregator => config
                .database()
                .get_aggregator_batches_by_indexes(vec![batch_number as u64])
                .await
                .ok()
                .and_then(|b| b.first().map(|b| b.end_block as f64))
                .unwrap_or(batch_number),
            JobType::SnosRun => config
                .database()
                .get_snos_batches_by_indices(vec![batch_number as u64])
                .await
                .ok()
                .and_then(|b| b.first().map(|b| b.end_block as f64))
                .unwrap_or(batch_number),
            _ => batch_number,
        }
    }

    fn record_success_metrics(job: &JobItem, operation: &str, external_id: ExternalId, duration: Duration) {
        let attributes = vec![
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", operation.to_string()),
        ];
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);

        if let Ok(block_number) = Self::get_block_number_from_ids(&job.job_type, &job.internal_id, external_id) {
            ORCHESTRATOR_METRICS.block_gauge.record(block_number, &attributes);
        }
    }

    fn record_verify_metrics(
        job: &JobItem,
        final_status: Option<JobStatus>,
        verification_status: &JobVerificationStatus,
        duration: Duration,
    ) {
        let mut attributes = vec![
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "verify_job"),
            KeyValue::new("operation_verification_status", format!("{:?}", verification_status)),
        ];
        if let Some(status) = final_status {
            attributes.push(KeyValue::new("operation_job_status", format!("{}", status)));
        }
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.jobs_response_time.record(duration.as_secs_f64(), &attributes);

        if let Ok(block_number) =
            Self::get_block_number_from_ids(&job.job_type, &job.internal_id, job.external_id.clone())
        {
            ORCHESTRATOR_METRICS.block_gauge.record(block_number, &attributes);
        }
    }

    fn get_block_number_from_ids(
        job_type: &JobType,
        internal_id: &str,
        external_id: ExternalId,
    ) -> Result<f64, JobError> {
        if let JobType::StateTransition = job_type {
            parse_string(
                external_id
                    .unwrap_string()
                    .map_err(|e| JobError::Other(OtherError::from(format!("Could not parse string: {e}"))))?,
            )
        } else {
            parse_string(internal_id)
        }
    }

    /// Maps WorkerTriggerType to the appropriate JobTrigger implementation
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
