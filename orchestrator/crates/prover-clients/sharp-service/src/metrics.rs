use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Histogram};
use opentelemetry::KeyValue;
use orchestrator_utils::metrics::lib::{
    register_counter_metric_instrument, register_histogram_metric_instrument, Metrics,
};
use orchestrator_utils::register_metric;

register_metric!(SHARP_METRICS, SharpMetrics);

/// Metrics for SHARP API calls exported to OTEL.
///
/// Labels:
/// - `operation`: API endpoint (`add_job`, `add_applicative_job`, `get_status`, `get_proof`)
/// - `success`: `"true"` / `"false"`
/// - `error_type`: error variant name when `success=false`, `"none"` otherwise
pub struct SharpMetrics {
    /// Duration of SHARP API calls in seconds.
    pub api_duration_seconds: Histogram<f64>,
    /// Total number of SHARP API calls.
    pub api_calls_total: Counter<f64>,
    /// Total number of retry attempts across all calls.
    pub retries_total: Counter<f64>,
    /// Total number of calls that required at least one retry.
    pub calls_with_retry_total: Counter<f64>,
    /// Idempotency short-circuits (job already existed on SHARP).
    pub idempotency_hits_total: Counter<f64>,
    /// Distribution of SHARP job statuses observed during polling.
    pub job_status_total: Counter<f64>,
}

impl Metrics for SharpMetrics {
    fn register() -> Self {
        let meter = global::meter("sharp_service");

        Self {
            api_duration_seconds: register_histogram_metric_instrument(
                &meter,
                "sharp_api_duration_seconds".to_string(),
                "Duration of SHARP API calls".to_string(),
                "s".to_string(),
            ),
            api_calls_total: register_counter_metric_instrument(
                &meter,
                "sharp_api_calls_total".to_string(),
                "Total number of SHARP API calls".to_string(),
                "calls".to_string(),
            ),
            retries_total: register_counter_metric_instrument(
                &meter,
                "sharp_retries_total".to_string(),
                "Total number of SHARP API retry attempts".to_string(),
                "retries".to_string(),
            ),
            calls_with_retry_total: register_counter_metric_instrument(
                &meter,
                "sharp_calls_with_retry_total".to_string(),
                "Total number of SHARP calls that required at least one retry".to_string(),
                "calls".to_string(),
            ),
            idempotency_hits_total: register_counter_metric_instrument(
                &meter,
                "sharp_idempotency_hits_total".to_string(),
                "Number of submit_task calls that short-circuited because the job already existed".to_string(),
                "hits".to_string(),
            ),
            job_status_total: register_counter_metric_instrument(
                &meter,
                "sharp_job_status_total".to_string(),
                "Distribution of SHARP job statuses observed during polling".to_string(),
                "observations".to_string(),
            ),
        }
    }
}

impl SharpMetrics {
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

    pub fn record_idempotency_hit(&self, task_type: &str) {
        self.idempotency_hits_total.add(1.0, &[KeyValue::new("task_type", task_type.to_string())]);
    }

    pub fn record_job_status(&self, task_type: &str, status: &str) {
        self.job_status_total.add(
            1.0,
            &[KeyValue::new("task_type", task_type.to_string()), KeyValue::new("status", status.to_string())],
        );
    }
}
