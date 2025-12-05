use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::KeyValue;
use orchestrator_utils::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use orchestrator_utils::register_metric;

register_metric!(ATLANTIC_METRICS, AtlanticMetrics);

/// Metrics for Atlantic API calls exported to OTEL
pub struct AtlanticMetrics {
    /// Duration of Atlantic API calls in seconds
    pub api_duration_seconds: Gauge<f64>,
    /// Total number of Atlantic API calls
    pub api_calls_total: Counter<f64>,
    /// Total bytes sent in requests
    pub request_bytes_total: Counter<f64>,
    /// Total bytes received in responses
    pub response_bytes_total: Counter<f64>,
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

        let request_bytes_total = register_counter_metric_instrument(
            &meter,
            "atlantic_request_bytes_total".to_string(),
            "Total request bytes sent to Atlantic API".to_string(),
            "bytes".to_string(),
        );

        let response_bytes_total = register_counter_metric_instrument(
            &meter,
            "atlantic_response_bytes_total".to_string(),
            "Total response bytes received from Atlantic API".to_string(),
            "bytes".to_string(),
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

        Self {
            api_duration_seconds,
            api_calls_total,
            request_bytes_total,
            response_bytes_total,
            retries_total,
            calls_with_retry_total,
        }
    }
}

impl AtlanticMetrics {
    /// Record a successful API call
    pub fn record_success(
        &self,
        operation: &str,
        duration_s: f64,
        request_bytes: u64,
        response_bytes: u64,
        retry_count: u32,
    ) {
        let attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("success", "true"),
            KeyValue::new("error_type", "none"),
        ];

        self.api_calls_total.add(1.0, &attrs);
        self.api_duration_seconds.record(duration_s, &attrs);

        let op_attr = [KeyValue::new("operation", operation.to_string())];
        self.request_bytes_total.add(request_bytes as f64, &op_attr);
        self.response_bytes_total.add(response_bytes as f64, &op_attr);

        if retry_count > 0 {
            self.retries_total.add(retry_count as f64, &op_attr);
            self.calls_with_retry_total.add(1.0, &op_attr);
        }
    }

    /// Record a failed API call
    pub fn record_failure(
        &self,
        operation: &str,
        duration_s: f64,
        request_bytes: u64,
        error_type: &str,
        retry_count: u32,
    ) {
        let attrs = [
            KeyValue::new("operation", operation.to_string()),
            KeyValue::new("success", "false"),
            KeyValue::new("error_type", error_type.to_string()),
        ];

        self.api_calls_total.add(1.0, &attrs);
        self.api_duration_seconds.record(duration_s, &attrs);

        let op_attr = [KeyValue::new("operation", operation.to_string())];
        self.request_bytes_total.add(request_bytes as f64, &op_attr);

        if retry_count > 0 {
            self.retries_total.add(retry_count as f64, &op_attr);
            self.calls_with_retry_total.add(1.0, &op_attr);
        }
    }
}
