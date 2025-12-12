use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::KeyValue;
use orchestrator_utils::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use orchestrator_utils::register_metric;

register_metric!(ATLANTIC_METRICS, AtlanticMetrics);

/// Metrics for Atlantic API calls exported to OTEL
///
/// These metrics track Atlantic API operations with detailed labels for monitoring:
/// - `operation`: The API operation name (add_job, get_job_status, create_bucket, etc.)
/// - `success`: Whether the operation succeeded (true/false)
/// - `error_type`: Type of error if failed (network_error, api_error, etc.)
///
/// Metrics are designed to answer questions like:
/// - How many proof generation jobs were submitted? (api_calls_total{operation="add_job"})
/// - What's the success rate for bucket creation? (api_calls_total{operation="create_bucket",success="true"})
/// - How many retries occurred for each operation? (retries_total{operation="add_job"})
/// - What's the average duration for each operation? (api_duration_seconds{operation="add_job"})
pub struct AtlanticMetrics {
    /// Duration of Atlantic API calls in seconds
    pub api_duration_seconds: Gauge<f64>,
    /// Total number of Atlantic API calls
    pub api_calls_total: Counter<f64>,
    /// Total number of retry attempts
    pub retries_total: Counter<f64>,
    /// Total number of calls that required at least one retry
    pub calls_with_retry_total: Counter<f64>,
}

impl Metrics for AtlanticMetrics {
    fn register() -> Self {
        let meter = global::meter("atlantic_service");

        let api_duration_seconds = register_gauge_metric_instrument(
            &meter,
            "atlantic_api_duration_seconds".to_string(),
            "Duration of Atlantic API calls".to_string(),
            "s".to_string(),
        );

        let api_calls_total = register_counter_metric_instrument(
            &meter,
            "atlantic_api_calls_total".to_string(),
            "Total number of Atlantic API calls".to_string(),
            "calls".to_string(),
        );

        let retries_total = register_counter_metric_instrument(
            &meter,
            "atlantic_retries_total".to_string(),
            "Total number of Atlantic API retry attempts".to_string(),
            "retries".to_string(),
        );

        let calls_with_retry_total = register_counter_metric_instrument(
            &meter,
            "atlantic_calls_with_retry_total".to_string(),
            "Total number of calls that required at least one retry".to_string(),
            "calls".to_string(),
        );

        Self { api_duration_seconds, api_calls_total, retries_total, calls_with_retry_total }
    }
}

impl AtlanticMetrics {
    /// Record a successful API call
    ///
    /// Records metrics with the `operation` label, allowing job-type specific analysis:
    /// - `add_job` = Proof generation job submissions
    /// - `submit_l2_query` = L2 proof verification submissions
    /// - `create_bucket` / `close_bucket` = Batch/aggregation operations
    /// - `get_job_status` = Polling for job completion
    /// - `get_artifacts` / `get_proof_by_task_id` = Artifact downloads
    pub fn record_success(&self, operation: &str, duration_s: f64, retry_count: u32) {
        let attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("success", "true"),
            KeyValue::new("error_type", "none"),
        ];

        self.api_calls_total.add(1.0, &attrs);
        self.api_duration_seconds.record(duration_s, &attrs);

        if retry_count > 0 {
            let op_attr = [KeyValue::new("operation", operation.to_string())];
            self.retries_total.add(retry_count as f64, &op_attr);
            self.calls_with_retry_total.add(1.0, &op_attr);
        }
    }

    /// Record a failed API call
    ///
    /// Records metrics with the `operation` label, allowing job-type specific analysis:
    /// - `add_job` = Proof generation job submissions
    /// - `submit_l2_query` = L2 proof verification submissions
    /// - `create_bucket` / `close_bucket` = Batch/aggregation operations
    /// - `get_job_status` = Polling for job completion
    /// - `get_artifacts` / `get_proof_by_task_id` = Artifact downloads
    pub fn record_failure(&self, operation: &str, duration_s: f64, error_type: &str, retry_count: u32) {
        let attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("success", "false"),
            KeyValue::new("error_type", error_type.to_string()),
        ];

        self.api_calls_total.add(1.0, &attrs);
        self.api_duration_seconds.record(duration_s, &attrs);

        if retry_count > 0 {
            let op_attr = [KeyValue::new("operation", operation.to_string())];
            self.retries_total.add(retry_count as f64, &op_attr);
            self.calls_with_retry_total.add(1.0, &op_attr);
        }
    }
}
