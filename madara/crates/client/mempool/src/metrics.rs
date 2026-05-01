use crate::inner::MempoolStateSummary;
use mc_telemetry::{register_counter_metric_instrument, register_gauge_metric_instrument};
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

/// Metrics for the external-db outbox (WAL) append path.
///
/// These are emitted from the mempool acceptance path because the outbox append happens there.
/// They still use the `external_db_*` prefix since they are part of the external-db pipeline.
pub struct ExternalDbOutboxMetrics {
    pub outbox_writes: Counter<u64>,
    pub outbox_write_errors: Counter<u64>,
    pub outbox_strict_rejections: Counter<u64>,
    pub outbox_rollback_delete_errors: Counter<u64>,
}

impl ExternalDbOutboxMetrics {
    pub fn register() -> Self {
        let meter = global::meter("madara.external_db");

        Self {
            outbox_writes: meter
                .u64_counter("external_db_outbox_writes")
                .with_description("Outbox entries appended to RocksDB")
                .build(),
            // Keep name + description consistent with mc-external-db metrics.rs.
            outbox_write_errors: meter
                .u64_counter("external_db_outbox_write_errors")
                .with_description("Outbox write errors")
                .build(),
            outbox_strict_rejections: meter
                .u64_counter("external_db_outbox_strict_rejections")
                .with_description("Transactions rejected due to strict outbox write failure")
                .build(),
            outbox_rollback_delete_errors: meter
                .u64_counter("external_db_outbox_rollback_delete_errors")
                .with_description("Outbox rollback delete failures when mempool insertion fails")
                .build(),
        }
    }
}
