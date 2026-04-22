use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge};
use orchestrator_utils::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use orchestrator_utils::register_metric;

register_metric!(MOCK_METRICS, MockProverMetrics);

/// Metrics for Mock prover operations.
pub struct MockProverMetrics {
    /// Duration of fact registration (ensure_registered) calls.
    pub fact_registration_duration_seconds: Gauge<f64>,
    /// Fact already registered on-chain (no tx sent).
    pub fact_already_registered_total: Counter<f64>,
    /// New fact registered on-chain (tx mined).
    pub fact_newly_registered_total: Counter<f64>,
    /// Fact registration errors.
    pub fact_registration_errors_total: Counter<f64>,
    /// Duration of isValid checks during verification.
    pub is_valid_check_duration_seconds: Gauge<f64>,
}

impl Metrics for MockProverMetrics {
    fn register() -> Self {
        let meter = global::meter("mock_service");

        Self {
            fact_registration_duration_seconds: register_gauge_metric_instrument(
                &meter,
                "mock_fact_registration_duration_seconds".to_string(),
                "Duration of fact registration calls".to_string(),
                "s".to_string(),
            ),
            fact_already_registered_total: register_counter_metric_instrument(
                &meter,
                "mock_fact_already_registered_total".to_string(),
                "Number of facts already registered (no tx sent)".to_string(),
                "facts".to_string(),
            ),
            fact_newly_registered_total: register_counter_metric_instrument(
                &meter,
                "mock_fact_newly_registered_total".to_string(),
                "Number of new facts registered on-chain".to_string(),
                "facts".to_string(),
            ),
            fact_registration_errors_total: register_counter_metric_instrument(
                &meter,
                "mock_fact_registration_errors_total".to_string(),
                "Number of fact registration errors".to_string(),
                "errors".to_string(),
            ),
            is_valid_check_duration_seconds: register_gauge_metric_instrument(
                &meter,
                "mock_is_valid_check_duration_seconds".to_string(),
                "Duration of isValid checks during verification".to_string(),
                "s".to_string(),
            ),
        }
    }
}
