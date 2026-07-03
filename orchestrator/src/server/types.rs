use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, SnosBatch, SnosBatchStatus};
use crate::types::jobs::types::{JobStatus, JobType};
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::{BlockRouteError, JobRouteError};

/// Represents a job identifier in API requests.
///
/// This struct is used to deserialize job IDs from incoming HTTP requests,
/// particularly in path parameters.
///
/// # Examples
/// ```
/// use orchestrator::server::types::JobId;
/// let job_id = JobId { id: "123e4567-e89b-12d3-a456-426614174000".to_string() };
/// ```
#[derive(Deserialize)]
pub struct JobId {
    /// The string representation of the job's UUID
    pub id: String,
}

/// Represents query parameters for filtering jobs by status.
#[derive(Deserialize)]
pub struct JobStatusQuery {
    pub status: JobStatus,
}

#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum BatchSortOrder {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Deserialize)]
pub struct BatchQuery {
    pub index: Option<u64>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    #[serde(default)]
    pub sort: BatchSortOrder,
}

/// Represents query parameters for priority queue selection.
#[derive(Deserialize)]
pub struct PriorityQuery {
    /// Whether to use the priority queue (defaults to false)
    #[serde(default)]
    pub priority: bool,
}

/// Represents a standardized API response structure.
///
/// This struct provides a consistent format for all API responses, including
/// both successful operations and errors. It implements serialization for
/// converting responses to JSON.
///
/// # Fields
/// * `success` - Indicates whether the operation was successful
/// * `message` - Optional message providing additional details (typically used for errors)
///
/// # Examples
/// ```
/// // Success response
/// use orchestrator::server::types::ApiResponse;
/// let response: ApiResponse<()> = ApiResponse::success(None);
/// assert_eq!(response.success, true);
/// assert_eq!(response.message, None);
///
/// // Error response
/// let response = ApiResponse::error("Invalid job ID".to_string());
/// assert_eq!(response.success, false);
/// assert_eq!(response.message, Some("Invalid job ID".to_string()));
/// ```
#[derive(Serialize, Deserialize)]
pub struct ApiResponse<T = ()> {
    /// Indicates if the operation was successful
    pub success: bool,
    /// Optional data payload
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
    /// Optional message, typically used for error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ApiResponse<()> {
    /// Creates an error response with the specified message.
    ///
    /// # Arguments
    /// * `message` - The error message to include in the response
    ///
    /// # Returns
    /// Returns an `ApiResponse` with `success` set to `false` and the provided message.
    ///
    /// # Examples
    /// ```
    /// use orchestrator::server::types::ApiResponse;
    /// let response = ApiResponse::error("Operation failed".to_string());
    /// assert_eq!(response.success, false);
    /// assert_eq!(response.message, Some("Operation failed".to_string()));
    /// ```
    pub fn error(message: String) -> Self {
        Self { success: false, data: None, message: Some(message) }
    }
}

impl<T> ApiResponse<T> {
    /// Creates a successful response with optional data and message.
    pub fn success_with_data(data: T, message: Option<String>) -> Self {
        Self { success: true, data: Some(data), message }
    }

    /// Creates a successful response with no message.
    ///
    /// # Returns
    /// Returns an `ApiResponse` with `success` set to `true` and no message.
    ///
    /// # Examples
    /// ```
    /// use orchestrator::server::types::ApiResponse;
    /// let response: ApiResponse<()> = ApiResponse::success(None);
    /// assert_eq!(response.success, true);
    /// ```
    pub fn success(message: Option<String>) -> Self {
        Self { success: true, data: None, message }
    }
}

/// Type alias for the result type used in job route handlers.
///
/// This type combines axum's `Response` type with our custom `JobRouteError`,
/// providing a consistent error handling pattern across all job-related routes.
///
/// # Examples
/// ```
/// use axum::Json;
/// use axum::response::IntoResponse;
/// use orchestrator::server::types::{ApiResponse, JobRouteResult};
/// use orchestrator::server::error::JobRouteError;
///
/// async fn handle_job() -> JobRouteResult {
///     // Success case
///     Ok(Json(ApiResponse::<()>::success(None)).into_response())
///     // Error case would be:
///     // Err(JobRouteError::NotFound("123".to_string()))
/// }
/// ```
pub type JobRouteResult = Result<Response<axum::body::Body>, JobRouteError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobStatusResponseItem {
    pub job_type: JobType,
    pub id: Uuid,
    pub status: JobStatus,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct JobStatusResponse {
    pub jobs: Vec<JobStatusResponseItem>,
}

pub type BlockRouteResult = Result<Response<axum::body::Body>, BlockRouteError>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockStatusResponse {
    pub batch_number: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SettlementJobTimestampsResponse {
    #[serde(default, with = "chrono::serde::ts_seconds_option")]
    pub process_started_at: Option<DateTime<Utc>>,
    #[serde(default, with = "chrono::serde::ts_seconds_option")]
    pub process_completed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "chrono::serde::ts_seconds_option")]
    pub verification_started_at: Option<DateTime<Utc>>,
    #[serde(default, with = "chrono::serde::ts_seconds_option")]
    pub verification_completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettlementJobResponseItem {
    pub job_type: JobType,
    pub id: Uuid,
    pub internal_id: u64,
    pub status: JobStatus,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
    pub timestamps: SettlementJobTimestampsResponse,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettlementSnosBatchResponse {
    pub index: u64,
    pub aggregator_batch_index: Option<u64>,
    pub start_block: u64,
    pub end_block: u64,
    pub status: SnosBatchStatus,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SettlementAggregatorBatchResponse {
    pub index: u64,
    pub start_block: u64,
    pub end_block: u64,
    #[serde(default)]
    pub aggregator_input_size_upper_bound: usize,
    pub status: AggregatorBatchStatus,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockSettlementStatusResponse {
    pub block_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snos_batch: Option<SettlementSnosBatchResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregator_batch: Option<SettlementAggregatorBatchResponse>,
    pub block_jobs: Vec<SettlementJobResponseItem>,
    pub aggregator_proof_jobs: Vec<SettlementJobResponseItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnosBatchMetricsResponse {
    pub state_diff_size: usize,
    pub sierra_gas: u64,
    pub proving_gas: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnosBatchDetailsResponse {
    #[serde(flatten)]
    pub batch: SettlementSnosBatchResponse,
    pub metrics: SnosBatchMetricsResponse,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AggregatorBatchDetailsResponse {
    #[serde(flatten)]
    pub batch: SettlementAggregatorBatchResponse,
    pub blob_len: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SnosBatchListResponse {
    pub batches: Vec<SnosBatchDetailsResponse>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AggregatorBatchListResponse {
    pub batches: Vec<AggregatorBatchDetailsResponse>,
}

impl From<&SnosBatch> for SettlementSnosBatchResponse {
    fn from(batch: &SnosBatch) -> Self {
        Self {
            index: batch.index,
            aggregator_batch_index: batch.aggregator_batch_index,
            start_block: batch.start_block,
            end_block: batch.end_block,
            status: batch.status.clone(),
            created_at: batch.created_at,
            updated_at: batch.updated_at,
        }
    }
}

impl From<&AggregatorBatch> for SettlementAggregatorBatchResponse {
    fn from(batch: &AggregatorBatch) -> Self {
        Self {
            index: batch.index,
            start_block: batch.start_block,
            end_block: batch.end_block,
            aggregator_input_size_upper_bound: batch.aggregator_input_size_upper_bound,
            status: batch.status.clone(),
            created_at: batch.created_at,
            updated_at: batch.updated_at,
        }
    }
}

impl From<&SnosBatch> for SnosBatchDetailsResponse {
    fn from(batch: &SnosBatch) -> Self {
        Self {
            batch: batch.into(),
            metrics: SnosBatchMetricsResponse {
                state_diff_size: batch.builtin_weights.state_diff_size,
                sierra_gas: batch.builtin_weights.sierra_gas.0,
                proving_gas: batch.builtin_weights.proving_gas.0,
            },
        }
    }
}

impl From<&AggregatorBatch> for AggregatorBatchDetailsResponse {
    fn from(batch: &AggregatorBatch) -> Self {
        Self { batch: batch.into(), blob_len: batch.blob_len }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::StarknetVersion;
    use crate::types::batch::AggregatorBatchWeights;
    use serde_json::json;

    #[test]
    fn aggregator_batch_response_includes_input_size_upper_bound() {
        let mut batch = AggregatorBatch::new(
            7,
            100,
            None,
            256,
            123_456,
            AggregatorBatchWeights::new(10, 20),
            StarknetVersion::V0_14_2,
        );
        batch.end_block = 109;
        batch.status = AggregatorBatchStatus::Closed;

        let response = SettlementAggregatorBatchResponse::from(&batch);

        assert_eq!(response.aggregator_input_size_upper_bound, 123_456);
    }

    #[test]
    fn aggregator_batch_response_deserializes_without_input_size_upper_bound() {
        let response: SettlementAggregatorBatchResponse = serde_json::from_value(json!({
            "index": 7,
            "start_block": 100,
            "end_block": 109,
            "status": "Closed",
            "created_at": 1_700_000_000,
            "updated_at": 1_700_000_010
        }))
        .unwrap();

        assert_eq!(response.aggregator_input_size_upper_bound, 0);
    }
}
