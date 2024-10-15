use mc_analytics::{register_counter_metric_instrument, register_gauge_metric_instrument, register_metric, Metrics};
use once_cell::sync::Lazy;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, KeyValue};

register_metric!(MEMPOOL_METRICS, MempoolMetrics);

pub struct MempoolMetrics {
    pub block_gauge: Gauge<u64>,
    pub block_counter: Counter<u64>,
    pub transaction_counter: Counter<u64>,
    pub accepted_transaction_counter: Counter<u64>,
}

impl Metrics for MempoolMetrics {
    fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "mempool")];
        let mempool_meter = global::meter_with_version(
            "crates.mempool.opentelemetry",
            // TODO: Unsure of these settings, come back
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let block_gauge = register_gauge_metric_instrument(
            &mempool_meter,
            "block_produced_no".to_string(),
            "A gauge to show block state at given time".to_string(),
            "block".to_string(),
        );
        let block_counter = register_counter_metric_instrument(
            &mempool_meter,
            "block_produced_count".to_string(),
            "A counter to show block state at given time".to_string(),
            "block".to_string(),
        );
        let transaction_counter = register_counter_metric_instrument(
            &mempool_meter,
            "transaction_count".to_string(),
            "A counter to show transaction state for the given block".to_string(),
            "transaction".to_string(),
        );

        let accepted_transaction_counter = register_counter_metric_instrument(
            &mempool_meter,
            "accepted_transaction_count".to_string(),
            "A counter to show accepted transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        Self { block_gauge, block_counter, transaction_counter, accepted_transaction_counter }
    }
}
