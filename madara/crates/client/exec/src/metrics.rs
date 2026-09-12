//! Metrics for transaction execution in Madara.
//!
//! This module provides execution time tracking for individual transactions.
//! Metrics are exported via OpenTelemetry (OTEL) for integration with Prometheus/OTLP.

use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram, ObservableCounter};
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
    /// Process-wide hash cache counters are observed from dependency-owned atomics.
    _hash_cache_total_calls_counter: ObservableCounter<u64>,
    _hash_cache_hits_counter: ObservableCounter<u64>,
    _hash_cache_misses_counter: ObservableCounter<u64>,
    _hash_cache_capacity_clears_counter: ObservableCounter<u64>,
    /// Process-wide Blockifier executor counters are observed from dependency-owned atomics.
    _blockifier_transactions_counter: ObservableCounter<u64>,
    _blockifier_committed_transactions_counter: ObservableCounter<u64>,
    _blockifier_execution_attempts_counter: ObservableCounter<u64>,
    _blockifier_validation_attempts_counter: ObservableCounter<u64>,
    _blockifier_aborts_counter: ObservableCounter<u64>,
    _blockifier_commit_phase_aborts_counter: ObservableCounter<u64>,
}

struct HashCacheInstruments {
    total_calls: ObservableCounter<u64>,
    hits: ObservableCounter<u64>,
    misses: ObservableCounter<u64>,
    capacity_clears: ObservableCounter<u64>,
}

/// Registers process-wide hash-cache observers backed by dependency-owned counters.
/// Each callback reads current totals and preserves the existing per-kind labels.
fn register_hash_cache_instruments(meter: &opentelemetry::metrics::Meter) -> HashCacheInstruments {
    let hash_cache_total_calls_counter = meter
        .u64_observable_counter("exec_hash_cache_calls_total")
        .with_description("Execution hash cache calls by hash kind")
        .with_unit("call")
        .with_callback(|observer| {
            for cache in starknet_api::hash_cache_metrics() {
                observer.observe(cache.total_calls, &[KeyValue::new("kind", cache.kind.as_str())]);
            }
            let cache = mc_class_exec::pedersen_cache_metrics();
            observer.observe(cache.total_calls, &[KeyValue::new("kind", "cairo_native_pedersen")]);
        })
        .build();

    let hash_cache_hits_counter = meter
        .u64_observable_counter("exec_hash_cache_hits_total")
        .with_description("Execution hash cache hits by hash kind")
        .with_unit("hit")
        .with_callback(|observer| {
            for cache in starknet_api::hash_cache_metrics() {
                observer.observe(cache.hits, &[KeyValue::new("kind", cache.kind.as_str())]);
            }
            let cache = mc_class_exec::pedersen_cache_metrics();
            observer.observe(cache.hits, &[KeyValue::new("kind", "cairo_native_pedersen")]);
        })
        .build();

    let hash_cache_misses_counter = meter
        .u64_observable_counter("exec_hash_cache_misses_total")
        .with_description("Execution hash cache misses by hash kind")
        .with_unit("miss")
        .with_callback(|observer| {
            for cache in starknet_api::hash_cache_metrics() {
                observer.observe(cache.misses, &[KeyValue::new("kind", cache.kind.as_str())]);
            }
            let cache = mc_class_exec::pedersen_cache_metrics();
            observer.observe(cache.misses, &[KeyValue::new("kind", "cairo_native_pedersen")]);
        })
        .build();

    let hash_cache_capacity_clears_counter = meter
        .u64_observable_counter("exec_hash_cache_capacity_clears_total")
        .with_description("Execution hash cache clears caused by reaching configured capacity")
        .with_unit("clear")
        .with_callback(|observer| {
            for cache in starknet_api::hash_cache_metrics() {
                observer.observe(cache.capacity_clears, &[KeyValue::new("kind", cache.kind.as_str())]);
            }
            let cache = mc_class_exec::pedersen_cache_metrics();
            observer.observe(cache.capacity_clears, &[KeyValue::new("kind", "cairo_native_pedersen")]);
        })
        .build();

    HashCacheInstruments {
        total_calls: hash_cache_total_calls_counter,
        hits: hash_cache_hits_counter,
        misses: hash_cache_misses_counter,
        capacity_clears: hash_cache_capacity_clears_counter,
    }
}

struct BlockifierInstruments {
    transactions: ObservableCounter<u64>,
    committed_transactions: ObservableCounter<u64>,
    execution_attempts: ObservableCounter<u64>,
    validation_attempts: ObservableCounter<u64>,
    aborts: ObservableCounter<u64>,
    commit_phase_aborts: ObservableCounter<u64>,
}

/// Registers process-wide Blockifier observers backed by its executor counters.
/// Callbacks expose submitted, committed, speculative, and abort totals unchanged.
fn register_blockifier_instruments(meter: &opentelemetry::metrics::Meter) -> BlockifierInstruments {
    let blockifier_transactions_counter = meter
        .u64_observable_counter("blockifier_transactions_total")
        .with_description("Transactions submitted to completed Blockifier execution chunks")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().transactions, &[]);
        })
        .build();

    let blockifier_committed_transactions_counter = meter
        .u64_observable_counter("blockifier_committed_transactions_total")
        .with_description("Transactions in the committed prefix of completed Blockifier execution chunks")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().committed_transactions, &[]);
        })
        .build();

    let blockifier_execution_attempts_counter = meter
        .u64_observable_counter("blockifier_execution_attempts_total")
        .with_description("Blockifier transaction execution attempts, including speculative re-execution")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().execution_attempts, &[]);
        })
        .build();

    let blockifier_validation_attempts_counter = meter
        .u64_observable_counter("blockifier_validation_attempts_total")
        .with_description("Blockifier speculative transaction validation attempts")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().validation_attempts, &[]);
        })
        .build();

    let blockifier_aborts_counter = meter
        .u64_observable_counter("blockifier_aborts_total")
        .with_description("Blockifier speculative executions invalidated and scheduled for re-execution")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().aborts, &[]);
        })
        .build();

    let blockifier_commit_phase_aborts_counter = meter
        .u64_observable_counter("blockifier_commit_phase_aborts_total")
        .with_description("Blockifier aborts first discovered while attempting to commit")
        .with_callback(|observer| {
            observer.observe(blockifier::metrics::transaction_executor_metrics().commit_phase_aborts, &[]);
        })
        .build();

    BlockifierInstruments {
        transactions: blockifier_transactions_counter,
        committed_transactions: blockifier_committed_transactions_counter,
        execution_attempts: blockifier_execution_attempts_counter,
        validation_attempts: blockifier_validation_attempts_counter,
        aborts: blockifier_aborts_counter,
        commit_phase_aborts: blockifier_commit_phase_aborts_counter,
    }
}

/// Registers a counter from borrowed descriptor strings.
/// This keeps the top-level metric table compact and preserves its identity.
fn register_counter(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Counter<u64> {
    register_counter_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers a gauge from borrowed descriptor strings.
/// This keeps the top-level metric table compact and preserves its identity.
fn register_gauge(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Gauge<u64> {
    register_gauge_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers a histogram from borrowed descriptor strings.
/// This keeps the top-level metric table compact and preserves its identity.
fn register_histogram(
    meter: &opentelemetry::metrics::Meter,
    name: &str,
    description: &str,
    unit: &str,
) -> Histogram<f64> {
    register_histogram_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

impl ExecutionMetrics {
    /// Registers execution, read-cache, hash-cache, and Blockifier metrics.
    /// Observer callbacks remain backed by the same process-wide dependency counters.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.exec.opentelemetry")
                .with_attributes([KeyValue::new("crate", "exec")])
                .build(),
        );
        let hash_cache = register_hash_cache_instruments(&meter);
        let blockifier = register_blockifier_instruments(&meter);

        Self {
            tx_execution_time_histogram: register_histogram(
                &meter,
                "tx_execution_time_ms",
                "Time taken to execute individual transactions",
                "ms",
            ),
            read_cache_hits_counter: register_counter(
                &meter,
                "exec_read_cache_hits_total",
                "Execution read cache hits",
                "hit",
            ),
            read_cache_misses_counter: register_counter(
                &meter,
                "exec_read_cache_misses_total",
                "Execution read cache misses",
                "miss",
            ),
            read_cache_size_bytes: register_gauge(
                &meter,
                "exec_read_cache_size_bytes",
                "Execution read cache size in bytes",
                "bytes",
            ),
            _hash_cache_total_calls_counter: hash_cache.total_calls,
            _hash_cache_hits_counter: hash_cache.hits,
            _hash_cache_misses_counter: hash_cache.misses,
            _hash_cache_capacity_clears_counter: hash_cache.capacity_clears,
            _blockifier_transactions_counter: blockifier.transactions,
            _blockifier_committed_transactions_counter: blockifier.committed_transactions,
            _blockifier_execution_attempts_counter: blockifier.execution_attempts,
            _blockifier_validation_attempts_counter: blockifier.validation_attempts,
            _blockifier_aborts_counter: blockifier.aborts,
            _blockifier_commit_phase_aborts_counter: blockifier.commit_phase_aborts,
        }
    }

    /// Records one transaction execution duration with transaction-type and context labels.
    /// Callers supply milliseconds to match the exported metric unit.
    pub fn record_tx_execution_time(&self, duration_ms: f64, tx_type: &str, context: &str) {
        self.tx_execution_time_histogram.record(
            duration_ms,
            &[KeyValue::new("tx_type", tx_type.to_string()), KeyValue::new("context", context.to_string())],
        );
    }

    /// Records a read-cache hit under the supplied cache-kind label.
    /// Tests mirror the counter so assertions do not depend on exporter collection.
    pub fn record_read_cache_hit(&self, kind: &'static str) {
        #[cfg(test)]
        test_counters::READ_CACHE_HITS_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.read_cache_hits_counter.add(1, &[KeyValue::new("kind", kind)]);
    }

    /// Records a read-cache miss under the supplied cache-kind label.
    /// Tests mirror the counter so assertions do not depend on exporter collection.
    pub fn record_read_cache_miss(&self, kind: &'static str) {
        #[cfg(test)]
        test_counters::READ_CACHE_MISSES_TOTAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.read_cache_misses_counter.add(1, &[KeyValue::new("kind", kind)]);
    }

    /// Records the current execution read-cache size in bytes.
    /// Tests mirror both the latest value and the number of observations.
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

/// Returns the lazily initialized global execution metric set.
/// Initialization occurs on first access and is shared process-wide.
pub fn metrics() -> &'static ExecutionMetrics {
    &METRICS
}

/// Helper to time transaction execution in RPC context.
pub struct TxExecutionTimer {
    start: Instant,
}

impl TxExecutionTimer {
    /// Starts a timer for one RPC-side transaction execution.
    /// Call `finish` with the transaction type to publish the observation.
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    /// Stops the timer and records its duration under the RPC execution context.
    /// Consuming self prevents the same execution from being recorded twice.
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
