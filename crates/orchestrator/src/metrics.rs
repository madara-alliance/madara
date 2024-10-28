use once_cell;
use once_cell::sync::Lazy;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, KeyValue};
use utils::metrics::lib::{register_gauge_metric_instrument, Metrics};
use utils::register_metric;

register_metric!(ORCHESTRATOR_METRICS, OrchestratorMetrics);

pub struct OrchestratorMetrics {
    pub block_gauge: Gauge<f64>,
}

impl Metrics for OrchestratorMetrics {
    fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "orchestrator")];
        let orchestrator_meter = global::meter_with_version(
            "crates.orchestrator.opentelemetry",
            // TODO: Unsure of these settings, come back
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        // Register all instruments
        let block_gauge = register_gauge_metric_instrument(
            &orchestrator_meter,
            "block_state".to_string(),
            "A gauge to show block state at given time".to_string(),
            "block".to_string(),
        );

        Self { block_gauge }
    }
}
