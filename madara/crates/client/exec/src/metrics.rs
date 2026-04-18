//! Metrics for transaction execution in Madara.
//!
//! This module provides execution time tracking for individual transactions.
//! Metrics are exported via OpenTelemetry (OTEL) for integration with Prometheus/OTLP.

use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use starknet_api::executable_transaction::TransactionType;
use std::time::Instant;

/// Test-only counters for verifying metrics in unit tests.
#[cfg(test)]
pub mod test_counters {
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub static READ_CACHE_HITS_TOTAL: AtomicU64 = AtomicU64::new(0);
    pub static READ_CACHE_MISSES_TOTAL: AtomicU64 = AtomicU64::new(0);
    pub static READ_CACHE_SIZE_LAST: AtomicU64 = AtomicU64::new(0);
    pub static READ_CACHE_SIZE_RECORDS: AtomicU64 = AtomicU64::new(0);

    pub fn acquire_and_reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        reset_all();
        guard
    }

    pub fn reset_all() {
        READ_CACHE_HITS_TOTAL.store(0, Ordering::Relaxed);
        READ_CACHE_MISSES_TOTAL.store(0, Ordering::Relaxed);
        READ_CACHE_SIZE_LAST.store(0, Ordering::Relaxed);
        READ_CACHE_SIZE_RECORDS.store(0, Ordering::Relaxed);
    }
}

/// Transaction type labels for metrics.
pub mod tx_type_label {
    pub const INVOKE: &str = "invoke";
    pub const DECLARE: &str = "declare";
    pub const DEPLOY_ACCOUNT: &str = "deploy_account";
    pub const L1_HANDLER: &str = "l1_handler";
}

/// Execution context labels for metrics.
pub mod context_label {
    /// RPC re-execution (tracing, fee estimation, simulation).
    pub const RPC: &str = "rpc";
    /// Real block production execution.
    pub const PRODUCTION: &str = "production";
}

/// Convert TransactionType to a metric label string.
pub fn tx_type_to_label(tx_type: TransactionType) -> &'static str {
    match tx_type {
        TransactionType::InvokeFunction => tx_type_label::INVOKE,
        TransactionType::Declare => tx_type_label::DECLARE,
        TransactionType::DeployAccount => tx_type_label::DEPLOY_ACCOUNT,
        TransactionType::L1Handler => tx_type_label::L1_HANDLER,
    }
}

/// Metrics for transaction execution operations.
#[derive(Debug)]
pub struct ExecutionMetrics {
    /// Histogram tracking per-transaction execution time in milliseconds.
    tx_execution_time_histogram: Histogram<f64>,
    /// Cache hits for execution read cache.
    read_cache_hits_counter: Counter<u64>,
    /// Cache misses for execution read cache.
    read_cache_misses_counter: Counter<u64>,
    /// Current read cache size in bytes.
    read_cache_size_bytes: Gauge<u64>,
}

impl ExecutionMetrics {
    /// Register and initialize OTEL metrics.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.exec.opentelemetry")
                .with_attributes([KeyValue::new("crate", "exec")])
                .build(),
        );

        let tx_execution_time_histogram = register_histogram_metric_instrument(
            &meter,
            "tx_execution_time_ms".to_string(),
            "Time taken to execute individual transactions".to_string(),
            "ms".to_string(),
        );

        let read_cache_hits_counter = register_counter_metric_instrument(
            &meter,
            "exec_read_cache_hits_total".to_string(),
            "Execution read cache hits".to_string(),
            "hit".to_string(),
        );

        let read_cache_misses_counter = register_counter_metric_instrument(
            &meter,
            "exec_read_cache_misses_total".to_string(),
            "Execution read cache misses".to_string(),
            "miss".to_string(),
        );

        let read_cache_size_bytes = register_gauge_metric_instrument(
            &meter,
            "exec_read_cache_size_bytes".to_string(),
            "Execution read cache size in bytes".to_string(),
            "bytes".to_string(),
        );

        Self { tx_execution_time_histogram, read_cache_hits_counter, read_cache_misses_counter, read_cache_size_bytes }
    }

    /// Record transaction execution time with type and context labels.
    pub fn record_tx_execution_time(&self, duration_ms: f64, tx_type: &str, context: &str) {
        self.tx_execution_time_histogram.record(
            duration_ms,
            &[KeyValue::new("tx_type", tx_type.to_string()), KeyValue::new("context", context.to_string())],
        );
    }

    pub fn record_read_cache_hit(&self, kind: &'static str) {
        #[cfg(test)]
        test_counters::READ_CACHE_HITS_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.read_cache_hits_counter.add(1, &[KeyValue::new("kind", kind)]);
    }

    pub fn record_read_cache_miss(&self, kind: &'static str) {
        #[cfg(test)]
        test_counters::READ_CACHE_MISSES_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.read_cache_misses_counter.add(1, &[KeyValue::new("kind", kind)]);
    }

    pub fn record_read_cache_size_bytes(&self, size_bytes: u64) {
        #[cfg(test)]
        {
            test_counters::READ_CACHE_SIZE_LAST.store(size_bytes, std::sync::atomic::Ordering::Relaxed);
            test_counters::READ_CACHE_SIZE_RECORDS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        self.read_cache_size_bytes.record(size_bytes, &[]);
    }
}

/// Global metrics instance (initialized lazily on first access).
static METRICS: std::sync::LazyLock<ExecutionMetrics> = std::sync::LazyLock::new(ExecutionMetrics::register);

/// Get the global metrics instance.
pub fn metrics() -> &'static ExecutionMetrics {
    &METRICS
}

/// Helper to time transaction execution in RPC context.
pub struct TxExecutionTimer {
    start: Instant,
}

impl TxExecutionTimer {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    /// Finish timing and record metric with RPC context.
    pub fn finish(self, tx_type: TransactionType) {
        let duration_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        metrics().record_tx_execution_time(duration_ms, tx_type_to_label(tx_type), context_label::RPC);
    }
}

impl Default for TxExecutionTimer {
    fn default() -> Self {
        Self::new()
    }
}
