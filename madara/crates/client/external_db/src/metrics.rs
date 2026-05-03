//! OpenTelemetry metrics for external database integration.

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// Metrics for external database operations.
pub struct ExternalDbMetrics {
    /// Total transactions attempted to be written to external DB (includes duplicates).
    pub transactions_attempted: Counter<u64>,

    /// Total transactions successfully written to external DB
    pub transactions_written: Counter<u64>,

    /// Total duplicate transactions detected during Mongo inserts.
    pub transactions_duplicates: Counter<u64>,

    /// Transactions deleted by retention
    pub transactions_deleted: Counter<u64>,

    /// Transactions drained during startup sync.
    pub startup_sync_drained: Counter<u64>,

    /// Batch write latency histogram (seconds)
    pub write_latency: Histogram<f64>,

    /// MongoDB connection/write errors
    pub mongodb_write_errors: Counter<u64>,

    /// MongoDB availability gauge (1 = last Mongo operation succeeded, 0 = last Mongo operation failed).
    ///
    /// This is intended for alerting (for example, "Mongo is down") without needing to infer from retry counters.
    pub mongodb_up: Gauge<u64>,

    /// Outbox read errors (deserialization / RocksDB iterator failures).
    pub outbox_read_errors: Counter<u64>,

    /// Document conversion errors (ValidatedTransaction -> Mongo document).
    pub document_convert_errors: Counter<u64>,

    /// Current batch size
    pub batch_size: Gauge<u64>,

    /// Outbox write errors
    pub outbox_write_errors: Counter<u64>,

    /// Current outbox size
    pub outbox_size: Gauge<u64>,

    /// Outbox delete errors (RocksDB delete after successful Mongo write).
    pub outbox_delete_errors: Counter<u64>,

    /// Retry attempts due to MongoDB failures
    pub retry_count: Counter<u64>,

    /// Retention tick errors (L1 confirmation polling / Mongo deletions).
    pub retention_tick_errors: Counter<u64>,

    /// Retention lag in blocks (eligible confirmed blocks not yet processed).
    pub retention_lag_blocks: Gauge<u64>,

    /// Latest block confirmed on L1 (as seen by the backend).
    pub retention_latest_confirmed_block: Gauge<u64>,

    /// Latest block eligible for retention deletion (after retention delay).
    pub retention_eligible_latest_block: Gauge<u64>,
}

impl ExternalDbMetrics {
    /// Register metrics with the global meter provider.
    pub fn register() -> Self {
        let meter = opentelemetry::global::meter("madara.external_db");
        Self::register_with_meter(&meter)
    }

    /// Register metrics with a specific meter (useful for testing).
    pub fn register_with_meter(meter: &Meter) -> Self {
        Self {
            transactions_attempted: meter
                .u64_counter("external_db_transactions_attempted")
                .with_description("Total transactions attempted to be written to external DB")
                .build(),
            transactions_written: meter
                .u64_counter("external_db_transactions_written")
                .with_description("Total transactions written to external DB")
                .build(),
            transactions_duplicates: meter
                .u64_counter("external_db_transactions_duplicates")
                .with_description("Total duplicate transactions detected during Mongo inserts")
                .build(),
            transactions_deleted: meter
                .u64_counter("external_db_transactions_deleted")
                .with_description("Transactions deleted by retention")
                .build(),
            startup_sync_drained: meter
                .u64_counter("external_db_startup_sync_drained")
                .with_description("Transactions drained during external DB startup sync")
                .build(),
            write_latency: meter
                .f64_histogram("external_db_write_latency_seconds")
                .with_description("Batch write latency in seconds")
                .build(),
            mongodb_write_errors: meter
                .u64_counter("external_db_mongodb_write_errors")
                .with_description("MongoDB connection/write errors")
                .build(),
            mongodb_up: meter
                .u64_gauge("external_db_mongodb_up")
                .with_description("MongoDB availability gauge (1 = up, 0 = down)")
                .build(),
            outbox_read_errors: meter
                .u64_counter("external_db_outbox_read_errors")
                .with_description("Outbox read errors (deserialization / RocksDB iterator failures)")
                .build(),
            document_convert_errors: meter
                .u64_counter("external_db_document_convert_errors")
                .with_description("Document conversion errors (ValidatedTransaction -> Mongo document)")
                .build(),
            batch_size: meter.u64_gauge("external_db_batch_size").with_description("Current batch size").build(),
            outbox_write_errors: meter
                .u64_counter("external_db_outbox_write_errors")
                .with_description("Outbox write errors")
                .build(),
            outbox_size: meter.u64_gauge("external_db_outbox_size").with_description("Current outbox size").build(),
            outbox_delete_errors: meter
                .u64_counter("external_db_outbox_delete_errors")
                .with_description("Outbox delete errors (after successful Mongo write)")
                .build(),
            retry_count: meter
                .u64_counter("external_db_retry_count")
                .with_description("Retry attempts due to MongoDB failures")
                .build(),
            retention_tick_errors: meter
                .u64_counter("external_db_retention_tick_errors")
                .with_description("Retention tick errors (L1 polling / Mongo deletions)")
                .build(),
            retention_lag_blocks: meter
                .u64_gauge("external_db_retention_lag_blocks")
                .with_description("Retention lag in blocks (eligible confirmed blocks not yet processed)")
                .build(),
            retention_latest_confirmed_block: meter
                .u64_gauge("external_db_retention_latest_confirmed_block")
                .with_description("Latest block confirmed on L1")
                .build(),
            retention_eligible_latest_block: meter
                .u64_gauge("external_db_retention_eligible_latest_block")
                .with_description("Latest block eligible for retention deletion (after retention delay)")
                .build(),
        }
    }
}
