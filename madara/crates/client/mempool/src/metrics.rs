use mc_analytics::register_counter_metric_instrument;
use opentelemetry::metrics::Counter;
use opentelemetry::{global, KeyValue};

pub struct MempoolMetrics {
    pub accepted_transaction_counter: Counter<u64>,
}

impl MempoolMetrics {
    pub fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "mempool")];
        let mempool_meter = global::meter_with_version(
            "crates.mempool.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let accepted_transaction_counter = register_counter_metric_instrument(
            &mempool_meter,
            "accepted_transaction_count".to_string(),
            "A counter to show accepted transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        Self { accepted_transaction_counter }
    }
}
