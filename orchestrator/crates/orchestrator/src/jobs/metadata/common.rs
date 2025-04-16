//! Common metadata shared across all job types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Common metadata fields shared across all job types.
///
/// # Field Management
/// These fields are automatically managed by the job processing system and should not
/// be modified directly by workers or jobs. The system uses these fields to:
/// - Track processing and verification attempts
/// - Record completion timestamps
/// - Store failure information
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CommonMetadata {
    /// Number of times the job has been processed
    pub process_attempt_no: u16,
    /// Number of times the job has been retried after processing failures
    pub process_retry_attempt_no: u16,
    /// Number of times the job has been verified
    pub verification_attempt_no: u16,
    /// Number of times the job has been retried after verification failures
    pub verification_retry_attempt_no: u16,
    /// Timestamp when job processing started
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub process_started_at: Option<DateTime<Utc>>,
    /// Timestamp when job processing completed
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub process_completed_at: Option<DateTime<Utc>>,
    /// Timestamp when job verification started
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub verification_started_at: Option<DateTime<Utc>>,
    /// Timestamp when job verification completed
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub verification_completed_at: Option<DateTime<Utc>>,
    /// Reason for job failure if any
    pub failure_reason: Option<String>,
}
