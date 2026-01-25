//! OpenTelemetry metrics for external database integration.

use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

/// Metrics for external database operations.
pub struct ExternalDbMetrics {
    /// Total transactions successfully written to external DB
    pub transactions_written: Counter<u64>,

    /// Transactions deleted by retention
    pub transactions_deleted: Counter<u64>,

    /// Batch write latency histogram (seconds)
    pub write_latency: Histogram<f64>,

    /// MongoDB connection/write errors
    pub mongodb_write_errors: Counter<u64>,

    /// Current batch size
    pub batch_size: Gauge<u64>,

    /// Outbox write errors
    pub outbox_write_errors: Counter<u64>,

    /// Current outbox size
    pub outbox_size: Gauge<u64>,

    /// Retry attempts due to MongoDB failures
    pub retry_count: Counter<u64>,
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
            transactions_written: meter
                .u64_counter("external_db_transactions_written")
                .with_description("Total transactions written to external DB")
                .build(),
            transactions_deleted: meter
                .u64_counter("external_db_transactions_deleted")
                .with_description("Transactions deleted by retention")
                .build(),
            write_latency: meter
                .f64_histogram("external_db_write_latency_seconds")
                .with_description("Batch write latency in seconds")
                .build(),
            mongodb_write_errors: meter
                .u64_counter("external_db_mongodb_write_errors")
                .with_description("MongoDB connection/write errors")
                .build(),
            batch_size: meter
                .u64_gauge("external_db_batch_size")
                .with_description("Current batch size")
                .build(),
            outbox_write_errors: meter
                .u64_counter("external_db_outbox_write_errors")
                .with_description("Outbox write errors")
                .build(),
            outbox_size: meter.u64_gauge("external_db_outbox_size").with_description("Current outbox size").build(),
            retry_count: meter
                .u64_counter("external_db_retry_count")
                .with_description("Retry attempts due to MongoDB failures")
                .build(),
        }
    }
}
