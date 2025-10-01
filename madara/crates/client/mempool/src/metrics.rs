use mc_analytics::register_counter_metric_instrument;
use opentelemetry::metrics::Counter;
use opentelemetry::{global, InstrumentationScope, KeyValue};

pub struct MempoolMetrics {
    pub accepted_transaction_counter: Counter<u64>,
}

impl MempoolMetrics {
    pub fn register() -> Self {
        // Register meter
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.mempool.opentelemetry")
                .with_attributes([KeyValue::new("crate", "mempool")])
                .build(),
        );

        let accepted_transaction_counter = register_counter_metric_instrument(
            &meter,
            "accepted_transaction_count".to_string(),
            "A counter to show accepted transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        Self { accepted_transaction_counter }
    }
}
