use mc_analytics::register_histogram_metric_instrument;
use opentelemetry::metrics::Histogram;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::sync::LazyLock;

/// Commitment computation metrics
pub struct CommitmentMetrics {
    pub transaction_commitment_duration: Histogram<f64>,
    pub state_diff_commitment_duration: Histogram<f64>,
    pub events_commitment_duration: Histogram<f64>,
}

impl CommitmentMetrics {
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.block.opentelemetry")
                .with_attributes([KeyValue::new("crate", "block")])
                .build(),
        );

        let transaction_commitment_duration = register_histogram_metric_instrument(
            &meter,
            "transaction_commitment_duration_seconds".to_string(),
            "Time to compute transaction and receipt commitments".to_string(),
            "s".to_string(),
        );
        let state_diff_commitment_duration = register_histogram_metric_instrument(
            &meter,
            "state_diff_commitment_duration_seconds".to_string(),
            "Time to compute state diff commitment".to_string(),
            "s".to_string(),
        );
        let events_commitment_duration = register_histogram_metric_instrument(
            &meter,
            "events_commitment_duration_seconds".to_string(),
            "Time to compute events commitment".to_string(),
            "s".to_string(),
        );

        Self {
            transaction_commitment_duration,
            state_diff_commitment_duration,
            events_commitment_duration,
        }
    }
}

static METRICS: LazyLock<CommitmentMetrics> = LazyLock::new(CommitmentMetrics::register);

/// Get the global commitment metrics instance
pub fn metrics() -> &'static CommitmentMetrics {
    &METRICS
}
