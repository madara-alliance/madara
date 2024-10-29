use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, KeyValue};

pub struct BlockProductionMetrics {
    pub block_gauge: Gauge<u64>,
    pub transaction_gauge: Gauge<u64>,
}

impl BlockProductionMetrics {
    pub fn register() -> Self {
        // Register meter
        let common_scope_attributes = vec![KeyValue::new("crate", "block_production")];
        let mempool_meter = global::meter_with_version(
            "crates.block_production.opentelemetry",
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
        
        let transaction_gauge = register_gauge_metric_instrument(
            &mempool_meter,
            "transaction_no".to_string(),
            "A gauge to show transaction state for the given block".to_string(),
            "transaction".to_string(),
        );

        Self { block_gauge, transaction_gauge }
    }
}
