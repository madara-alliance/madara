use crate::types::jobs::types::{JobStatus, JobType};
use opentelemetry::metrics::Gauge;
use opentelemetry::KeyValue;

/// Tracks the current state of jobs by block number, job type, and status
pub struct JobStatusTracker {
    /// Gauge for tracking job states by block, type, and status
    pub job_status_gauge: Gauge<f64>,
    /// Gauge for tracking job transitions
    pub job_transition_gauge: Gauge<f64>,
    /// Gauge for tracking job details with job_id
    pub job_details_gauge: Gauge<f64>,
}

impl JobStatusTracker {
    /// Update the job status metric for a specific block and job type
    pub fn update_job_status(&self, block_number: u64, job_type: &JobType, status: &JobStatus, job_id: &str) {
        let labels = vec![
            KeyValue::new("block_number", block_number.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("operation_job_status", status.to_string()),
            KeyValue::new("job_id", job_id.to_string()),
        ];

        // Set gauge to 1 to indicate this combination exists
        self.job_status_gauge.record(1.0, &labels);

        // Also track as a transition
        self.record_transition(block_number, job_type, status, job_id);
    }

    /// Record a job state transition
    pub fn record_transition(&self, block_number: u64, job_type: &JobType, status: &JobStatus, job_id: &str) {
        let labels = vec![
            KeyValue::new("block_number", block_number.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("to_status", status.to_string()),
            KeyValue::new("job_id", job_id.to_string()),
        ];

        // Record the transition
        self.job_transition_gauge.record(1.0, &labels);
    }

    /// Update job details with full context
    pub fn update_job_details(
        &self,
        block_number: u64,
        job_type: &JobType,
        status: &JobStatus,
        job_id: &str,
        retry_count: i32,
        external_id: Option<String>,
    ) {
        let mut labels = vec![
            KeyValue::new("block_number", block_number.to_string()),
            KeyValue::new("operation_job_type", format!("{:?}", job_type)),
            KeyValue::new("operation_job_status", status.to_string()),
            KeyValue::new("job_id", job_id.to_string()),
            KeyValue::new("retry_count", retry_count.to_string()),
        ];

        if let Some(ext_id) = external_id {
            labels.push(KeyValue::new("external_id", ext_id));
        }

        // Record detailed job information
        self.job_details_gauge.record(block_number as f64, &labels);
    }

    /// Clear old job status entries (for completed jobs older than threshold)
    pub fn clear_old_entries(&self, block_threshold: u64) {
        // This would typically be implemented with a cleanup mechanism
        // to prevent unbounded metric growth
        let labels = vec![KeyValue::new("cleanup_threshold", block_threshold.to_string())];

        self.job_status_gauge.record(0.0, &labels);
    }
}

/// Helper function to get status color for dashboard
pub fn get_status_color(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Created => "blue",
        JobStatus::LockedForProcessing => "purple",
        JobStatus::PendingVerification => "yellow",
        JobStatus::Completed => "green",
        JobStatus::VerificationFailed => "orange",
        JobStatus::Failed => "red",
        JobStatus::VerificationTimeout => "dark_red",
        JobStatus::PendingRetry => "orange",
    }
}

/// Helper function to get status indicator for dashboard
pub fn get_status_icon(status: &JobStatus) -> &'static str {
    match status {
        JobStatus::Created => "[NEW]",
        JobStatus::LockedForProcessing => "[LOCKED]",
        JobStatus::PendingVerification => "[VERIFYING]",
        JobStatus::Completed => "[OK]",
        JobStatus::VerificationFailed => "[VERIFY_FAIL]",
        JobStatus::Failed => "[FAIL]",
        JobStatus::VerificationTimeout => "[TIMEOUT]",
        JobStatus::PendingRetry => "[RETRY]",
    }
}
