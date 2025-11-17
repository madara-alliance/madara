use mc_analytics::{register_counter_metric_instrument, register_gauge_metric_instrument};
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, InstrumentationScope, KeyValue};

pub struct MempoolMetrics {
    pub accepted_transaction_counter: Counter<u64>,
    // Inner mempool structures
    pub mempool_size: Gauge<u64>,
    pub mempool_ready_transactions: Gauge<u64>,
    pub mempool_num_accounts: Gauge<u64>,
    pub mempool_ready_queue_size: Gauge<u64>,
    pub mempool_timestamp_queue_size: Gauge<u64>,
    pub mempool_by_tx_hash_size: Gauge<u64>,
    pub mempool_eviction_queue_size: Gauge<u64>,
    // Preconfirmed transaction statuses
    pub preconfirmed_tx_statuses_size: Gauge<u64>,
    // Channel/pending metrics
    pub mempool_pending_consumer_locks: Gauge<u64>,
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

        let mempool_size = register_gauge_metric_instrument(
            &meter,
            "mempool_size".to_string(),
            "Total number of transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_ready_transactions = register_gauge_metric_instrument(
            &meter,
            "mempool_ready_transactions".to_string(),
            "Number of ready transactions in the mempool".to_string(),
            "transaction".to_string(),
        );

        let mempool_num_accounts = register_gauge_metric_instrument(
            &meter,
            "mempool_num_accounts".to_string(),
            "Number of accounts with transactions in the mempool".to_string(),
            "account".to_string(),
        );

        let mempool_ready_queue_size = register_gauge_metric_instrument(
            &meter,
            "mempool_ready_queue_size".to_string(),
            "Number of accounts in the ready queue".to_string(),
            "account".to_string(),
        );

        let mempool_timestamp_queue_size = register_gauge_metric_instrument(
            &meter,
            "mempool_timestamp_queue_size".to_string(),
            "Number of transactions in the timestamp queue".to_string(),
            "transaction".to_string(),
        );

        let mempool_by_tx_hash_size = register_gauge_metric_instrument(
            &meter,
            "mempool_by_tx_hash_size".to_string(),
            "Number of entries in the by_tx_hash index".to_string(),
            "transaction".to_string(),
        );

        let mempool_eviction_queue_size = register_gauge_metric_instrument(
            &meter,
            "mempool_eviction_queue_size".to_string(),
            "Number of accounts in the eviction queue".to_string(),
            "account".to_string(),
        );

        let preconfirmed_tx_statuses_size = register_gauge_metric_instrument(
            &meter,
            "preconfirmed_tx_statuses_size".to_string(),
            "Number of entries in the preconfirmed transactions statuses map".to_string(),
            "transaction".to_string(),
        );

        let mempool_pending_consumer_locks = register_gauge_metric_instrument(
            &meter,
            "mempool_pending_consumer_locks".to_string(),
            "Number of pending consumer lock requests (waiting for ready transactions)".to_string(),
            "lock".to_string(),
        );

        Self {
            accepted_transaction_counter,
            mempool_size,
            mempool_ready_transactions,
            mempool_num_accounts,
            mempool_ready_queue_size,
            mempool_timestamp_queue_size,
            mempool_by_tx_hash_size,
            mempool_eviction_queue_size,
            preconfirmed_tx_statuses_size,
            mempool_pending_consumer_locks,
        }
    }
}
