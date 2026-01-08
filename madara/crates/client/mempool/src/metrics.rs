use crate::inner::MempoolStateSummary;
use mc_analytics::{register_counter_metric_instrument, register_gauge_metric_instrument};
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, InstrumentationScope, KeyValue};

pub struct MempoolMetrics {
    pub accepted_transaction_counter: Counter<u64>,
    pub mempool_current_size: Gauge<u64>,
    pub mempool_ready_transactions: Gauge<u64>,
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

        let mempool_current_size = register_gauge_metric_instrument(
            &meter,
            "mempool_current_size".to_string(),
            "Current number of transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_ready_transactions = register_gauge_metric_instrument(
            &meter,
            "mempool_ready_transactions".to_string(),
            "Number of ready transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        Self { accepted_transaction_counter, mempool_current_size, mempool_ready_transactions }
    }

    pub fn record_mempool_state(&self, summary: &MempoolStateSummary) {
        self.mempool_current_size.record(summary.num_transactions as u64, &[]);
        self.mempool_ready_transactions.record(summary.ready_transactions as u64, &[]);
    }
}
