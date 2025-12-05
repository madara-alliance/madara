use chrono::Utc;
use opentelemetry::KeyValue;

use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;

/// Helper functions to record metrics at various points in the job lifecycle
/// These should be called from the existing service handlers without modifying the DB model
pub struct MetricsRecorder;

impl MetricsRecorder {
    /// Record metrics when a job is created and enters the queue
    pub fn record_job_created(job: &JobItem) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "create_job"),
            KeyValue::new("service_name", "${namespace}"),
        ];

        // Record that a job entered the queue
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);

        // TODO: Query DB to get current queue depth for this job type
        // This would require async context - implement in service layer
    }

    /// Record metrics when a job starts processing
    pub fn record_job_processing_started(job: &JobItem, queue_wait_time_seconds: f64) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_type", "process_job"),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        // Record queue wait time
        ORCHESTRATOR_METRICS.job_queue_wait_time.record(queue_wait_time_seconds, &attributes);

        // Record state transition
        let transition_attrs = [
            KeyValue::new("from_state", JobStatus::Created.to_string()),
            KeyValue::new("to_state", JobStatus::LockedForProcessing.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        ];
        ORCHESTRATOR_METRICS.job_state_transitions.add(1.0, &transition_attrs);
    }

    /// Record metrics when a job is retried
    pub fn record_job_retry(job: &JobItem, retry_reason: &str) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("retry_reason", retry_reason.to_string()),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.job_retry_count.add(1.0, &attributes);
    }

    /// Record metrics when job verification starts
    pub fn record_verification_started(job: &JobItem) {
        // Record state transition
        let transition_attrs = [
            KeyValue::new("from_state", JobStatus::LockedForProcessing.to_string()),
            KeyValue::new("to_state", JobStatus::PendingVerification.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        ];
        ORCHESTRATOR_METRICS.job_state_transitions.add(1.0, &transition_attrs);
    }

    /// Record metrics when a job completes successfully
    pub fn record_job_completed(job: &JobItem, e2e_duration_seconds: f64) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("operation_job_status", "Completed"),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        // Record E2E latency
        ORCHESTRATOR_METRICS.job_e2e_latency.record(e2e_duration_seconds, &attributes);

        // Record successful completion
        ORCHESTRATOR_METRICS.successful_job_operations.add(1.0, &attributes);

        // Record state transition
        let transition_attrs = [
            KeyValue::new("from_state", JobStatus::PendingVerification.to_string()),
            KeyValue::new("to_state", JobStatus::Completed.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
        ];
        ORCHESTRATOR_METRICS.job_state_transitions.add(1.0, &transition_attrs);
    }

    /// Record metrics when a job fails
    pub fn record_job_failed(job: &JobItem, failure_reason: &str) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("failure_reason", failure_reason.to_string()),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
        ORCHESTRATOR_METRICS.failed_jobs.add(1.0, &attributes);
    }

    /// Record metrics when a job times out
    pub fn record_job_timeout(job: &JobItem) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("timeout_type", "verification"),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.job_timeout_count.add(1.0, &attributes);
    }

    /// Record metrics when a job is abandoned after max retries
    pub fn record_job_abandoned(job: &JobItem, retry_count: i32) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("final_retry_count", retry_count.to_string()),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.job_abandoned_count.add(1.0, &attributes);
    }

    /// Record dependency wait time
    pub fn record_dependency_wait(job: &JobItem, wait_time_seconds: f64) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.dependency_wait_time.record(wait_time_seconds, &attributes);
    }

    /// Record proof generation time
    pub fn record_proof_generation_time(proof_type: &str, duration_seconds: f64) {
        let attributes = [KeyValue::new("proof_type", proof_type.to_string())];

        ORCHESTRATOR_METRICS.proof_generation_time.record(duration_seconds, &attributes);
    }

    /// Record settlement time
    pub fn record_settlement_time(job_type: &JobType, duration_seconds: f64) {
        let attributes =
            [KeyValue::new("operation_job_type", format!("{:?}", job_type)), KeyValue::new("settlement_layer", "L1")];

        ORCHESTRATOR_METRICS.settlement_time.record(duration_seconds, &attributes);
    }

    /// Record active jobs count (should be called when jobs change state)
    pub async fn record_active_jobs(count: f64) {
        let attributes = [KeyValue::new("status", "processing")];

        ORCHESTRATOR_METRICS.active_jobs_count.record(count, &attributes);
    }

    /// Record parallelism factor
    pub async fn record_parallelism_factor(factor: f64) {
        let attributes = [];

        ORCHESTRATOR_METRICS.job_parallelism_factor.record(factor, &attributes);
    }

    /// Check and record SLA breaches
    pub fn check_and_record_sla_breach(job: &JobItem, max_e2e_seconds: i64, sla_type: &str) {
        let age_seconds = Utc::now().signed_duration_since(job.created_at).num_seconds();

        if age_seconds > max_e2e_seconds {
            let attributes = [
                KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
                KeyValue::new("sla_type", sla_type.to_string()),
                KeyValue::new("breach_amount_seconds", (age_seconds - max_e2e_seconds).to_string()),
            ];

            ORCHESTRATOR_METRICS.sla_breach_count.add(1.0, &attributes);
        }
    }

    /// Record orphaned job detection
    pub fn record_orphaned_job(job: &JobItem) {
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", job.job_type)),
            KeyValue::new("internal_id", job.internal_id.clone()),
        ];

        ORCHESTRATOR_METRICS.orphaned_jobs.add(1.0, &attributes);
    }

    /// Record Atlantic API call metrics
    pub fn record_atlantic_api_call(
        operation: &str,
        duration_seconds: f64,
        data_size_bytes: u64,
        success: bool,
        retry_count: u32,
        error_type: Option<&str>,
    ) {
        // Record call duration
        let duration_attrs =
            [KeyValue::new("operation", operation.to_string()), KeyValue::new("success", success.to_string())];
        ORCHESTRATOR_METRICS.atlantic_api_call_duration.record(duration_seconds, &duration_attrs);

        // Record total calls
        let call_attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("status", if success { "success" } else { "error" }),
        ];
        ORCHESTRATOR_METRICS.atlantic_api_calls_total.add(1.0, &call_attrs);

        // Record errors if any
        if !success {
            let error_attrs = [
                KeyValue::new("operation", operation.to_string()),
                KeyValue::new("error_type", error_type.unwrap_or("unknown").to_string()),
            ];
            ORCHESTRATOR_METRICS.atlantic_api_errors_total.add(1.0, &error_attrs);
        }

        // Record retries if any
        if retry_count > 0 {
            let retry_attrs = [KeyValue::new("operation", operation.to_string())];
            ORCHESTRATOR_METRICS.atlantic_api_retries_total.add(retry_count as f64, &retry_attrs);
        }

        // Record data transfer
        if data_size_bytes > 0 {
            let data_attrs = [KeyValue::new("operation", operation.to_string()), KeyValue::new("direction", "request")];
            ORCHESTRATOR_METRICS.atlantic_data_transfer_bytes.add(data_size_bytes as f64, &data_attrs);
        }
    }

    /// Record Atlantic API response data size
    pub fn record_atlantic_response_size(operation: &str, data_size_bytes: u64) {
        if data_size_bytes > 0 {
            let attrs = [KeyValue::new("operation", operation.to_string()), KeyValue::new("direction", "response")];
            ORCHESTRATOR_METRICS.atlantic_data_transfer_bytes.add(data_size_bytes as f64, &attrs);
        }
    }
}
