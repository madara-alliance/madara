use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::LazyLock;

const BONSAI_DURATION_BUCKETS: &[f64] =
    &[0.0001, 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];
const BONSAI_COUNT_BUCKETS: &[f64] = &[
    0.0,
    1.0,
    2.0,
    5.0,
    10.0,
    25.0,
    50.0,
    100.0,
    250.0,
    500.0,
    1_000.0,
    2_500.0,
    5_000.0,
    10_000.0,
    25_000.0,
    50_000.0,
    100_000.0,
    250_000.0,
    500_000.0,
    1_000_000.0,
    5_000_000.0,
];
const BONSAI_BATCH_BYTES_BUCKETS: &[f64] = &[
    0.0,
    64.0,
    256.0,
    1_024.0,
    4_096.0,
    16_384.0,
    65_536.0,
    262_144.0,
    1_048_576.0,
    4_194_304.0,
    16_777_216.0,
    67_108_864.0,
    268_435_456.0,
    1_073_741_824.0,
];

/// Database metrics for close_block operations
pub struct DbMetrics {
    // Invariant/error counters
    pub head_projection_violation_count: Counter<u64>,
    /// Test-visible atomic mirror of `head_projection_violation_count` for delta assertions.
    #[cfg(test)]
    pub head_projection_violation_count_test: AtomicU64,

    // Histograms for percentile analysis
    pub apply_to_global_trie_duration: Histogram<f64>,
    pub contract_trie_root_duration: Histogram<f64>,
    pub class_trie_root_duration: Histogram<f64>,
    pub contract_storage_trie_commit_duration: Histogram<f64>,
    pub contract_trie_commit_duration: Histogram<f64>,
    pub class_trie_commit_duration: Histogram<f64>,
    pub block_commitments_compute_duration: Histogram<f64>,
    pub block_hash_compute_duration: Histogram<f64>,
    pub get_full_block_without_state_diff_duration: Histogram<f64>,
    pub db_write_block_parts_duration: Histogram<f64>,

    // Gauges for exact per-block values - Main 5 sequential operations
    pub get_full_block_without_state_diff_last: Gauge<f64>,
    pub block_commitments_compute_last: Gauge<f64>,
    pub apply_to_global_trie_last: Gauge<f64>,
    pub block_hash_compute_last: Gauge<f64>,
    pub db_write_block_parts_last: Gauge<f64>,

    // Gauges for Merklization Deep Dive
    pub contract_trie_root_last: Gauge<f64>,
    pub class_trie_root_last: Gauge<f64>,
    pub contract_storage_trie_commit_last: Gauge<f64>,
    pub contract_trie_commit_last: Gauge<f64>,
    pub class_trie_commit_last: Gauge<f64>,

    // Bonsai trie-log pruning. The same instruments are intentionally usable by
    // both the current point-delete implementation and a future range-delete
    // implementation, so a deployment can be compared without changing queries.
    pub bonsai_prefix_prune_operations: Counter<u64>,
    pub bonsai_prefix_prune_entries_scanned: Histogram<u64>,
    pub bonsai_prefix_prune_batch_operations: Histogram<u64>,
    pub bonsai_prefix_prune_batch_bytes: Histogram<u64>,
    pub bonsai_prefix_prune_duration: Histogram<f64>,
    pub bonsai_prefix_prune_last_revision: Gauge<u64>,
}

/// Registers a counter from borrowed descriptor strings.
/// This keeps the metric table compact without changing its exported identity.
fn register_counter(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Counter<u64> {
    register_counter_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers an integer gauge from borrowed descriptor strings.
/// This keeps the metric table compact without changing its exported identity.
fn register_u64_gauge(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Gauge<u64> {
    register_gauge_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers a floating-point gauge from borrowed descriptor strings.
/// Timing gauges use this variant so sub-second values retain their precision.
fn register_f64_gauge(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Gauge<f64> {
    register_gauge_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers a floating-point histogram from borrowed descriptor strings.
/// Default bucket behavior remains owned by the telemetry helper.
fn register_histogram(
    meter: &opentelemetry::metrics::Meter,
    name: &str,
    description: &str,
    unit: &str,
) -> Histogram<f64> {
    register_histogram_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Registers a u64 histogram with explicit pruning-count or batch-size boundaries.
/// The supplied static boundary table is copied into the OpenTelemetry builder.
fn register_u64_histogram(
    meter: &opentelemetry::metrics::Meter,
    name: &'static str,
    description: &'static str,
    boundaries: &[f64],
) -> Histogram<u64> {
    meter.u64_histogram(name).with_description(description).with_boundaries(boundaries.to_vec()).build()
}

/// Registers an f64 histogram with explicit duration boundaries and a unit.
/// The supplied static boundary table is copied into the OpenTelemetry builder.
fn register_f64_histogram(
    meter: &opentelemetry::metrics::Meter,
    name: &'static str,
    description: &'static str,
    unit: &'static str,
    boundaries: &[f64],
) -> Histogram<f64> {
    meter.f64_histogram(name).with_description(description).with_unit(unit).with_boundaries(boundaries.to_vec()).build()
}

/// Expands the declarative database metric table without hiding runtime control flow.
/// Keeping descriptors together makes name, unit, and bucket changes easy to review.
macro_rules! register_db_metrics {
    ($meter:expr) => {
        Self {
            head_projection_violation_count: register_counter(
                $meter,
                "head_projection_violation_total",
                "Number of head projection invariant violations",
                "violation",
            ),
            #[cfg(test)]
            head_projection_violation_count_test: AtomicU64::new(0),
            apply_to_global_trie_duration: register_histogram(
                $meter,
                "apply_to_global_trie_duration_seconds",
                "Total time for global trie merklization",
                "s",
            ),
            contract_trie_root_duration: register_histogram(
                $meter,
                "contract_trie_root_duration_seconds",
                "Time to compute contract trie root",
                "s",
            ),
            class_trie_root_duration: register_histogram(
                $meter,
                "class_trie_root_duration_seconds",
                "Time to compute class trie root",
                "s",
            ),
            contract_storage_trie_commit_duration: register_histogram(
                $meter,
                "contract_storage_trie_commit_duration_seconds",
                "Time to commit contract storage trie",
                "s",
            ),
            contract_trie_commit_duration: register_histogram(
                $meter,
                "contract_trie_commit_duration_seconds",
                "Time to commit contract trie",
                "s",
            ),
            class_trie_commit_duration: register_histogram(
                $meter,
                "class_trie_commit_duration_seconds",
                "Time to commit class trie",
                "s",
            ),
            block_commitments_compute_duration: register_histogram(
                $meter,
                "block_commitments_compute_duration_seconds",
                "Total time to compute block commitments",
                "s",
            ),
            block_hash_compute_duration: register_histogram(
                $meter,
                "block_hash_compute_duration_seconds",
                "Time to compute block hash",
                "s",
            ),
            get_full_block_without_state_diff_duration: register_histogram(
                $meter,
                "get_full_block_without_state_diff_duration_seconds",
                "Time to fetch full block without state diff",
                "s",
            ),
            db_write_block_parts_duration: register_histogram(
                $meter,
                "db_write_block_parts_duration_seconds",
                "Time to write block parts to database",
                "s",
            ),
            get_full_block_without_state_diff_last: register_f64_gauge(
                $meter,
                "get_full_block_without_state_diff_last_seconds",
                "Last block: time to fetch full block without state diff",
                "s",
            ),
            block_commitments_compute_last: register_f64_gauge(
                $meter,
                "block_commitments_compute_last_seconds",
                "Last block: time to compute block commitments",
                "s",
            ),
            apply_to_global_trie_last: register_f64_gauge(
                $meter,
                "apply_to_global_trie_last_seconds",
                "Last block: total time for global trie merklization",
                "s",
            ),
            block_hash_compute_last: register_f64_gauge(
                $meter,
                "block_hash_compute_last_seconds",
                "Last block: time to compute block hash",
                "s",
            ),
            db_write_block_parts_last: register_f64_gauge(
                $meter,
                "db_write_block_parts_last_seconds",
                "Last block: time to write block parts to database",
                "s",
            ),
            contract_trie_root_last: register_f64_gauge(
                $meter,
                "contract_trie_root_last_seconds",
                "Last block: time to compute contract trie root",
                "s",
            ),
            class_trie_root_last: register_f64_gauge(
                $meter,
                "class_trie_root_last_seconds",
                "Last block: time to compute class trie root",
                "s",
            ),
            contract_storage_trie_commit_last: register_f64_gauge(
                $meter,
                "contract_storage_trie_commit_last_seconds",
                "Last block: time to commit contract storage trie",
                "s",
            ),
            contract_trie_commit_last: register_f64_gauge(
                $meter,
                "contract_trie_commit_last_seconds",
                "Last block: time to commit contract trie",
                "s",
            ),
            class_trie_commit_last: register_f64_gauge(
                $meter,
                "class_trie_commit_last_seconds",
                "Last block: time to commit class trie",
                "s",
            ),
            bonsai_prefix_prune_operations: register_counter(
                $meter,
                "bonsai_prefix_prune_operations",
                "Number of Bonsai trie-log prefix removals",
                "",
            ),
            bonsai_prefix_prune_last_revision: register_u64_gauge(
                $meter,
                "bonsai_prefix_prune_last_revision",
                "Latest Bonsai trie-log revision fully removed by prefix",
                "",
            ),
            bonsai_prefix_prune_entries_scanned: register_u64_histogram(
                $meter,
                "bonsai_prefix_prune_entries_scanned",
                "RocksDB iterator entries examined by each Bonsai trie-log prefix removal",
                BONSAI_COUNT_BUCKETS,
            ),
            bonsai_prefix_prune_batch_operations: register_u64_histogram(
                $meter,
                "bonsai_prefix_prune_batch_operations",
                "RocksDB write-batch operations generated by each Bonsai trie-log prefix removal",
                BONSAI_COUNT_BUCKETS,
            ),
            bonsai_prefix_prune_batch_bytes: register_u64_histogram(
                $meter,
                "bonsai_prefix_prune_batch_bytes",
                "Serialized RocksDB write-batch size generated by each Bonsai trie-log prefix removal",
                BONSAI_BATCH_BYTES_BUCKETS,
            ),
            bonsai_prefix_prune_duration: register_f64_histogram(
                $meter,
                "bonsai_prefix_prune_duration_seconds",
                "Time spent scanning and deleting one Bonsai trie-log prefix",
                "s",
                BONSAI_DURATION_BUCKETS,
            ),
        }
    };
}

impl DbMetrics {
    /// Registers database timing, invariant, and trie-log pruning metrics.
    /// Names and histogram boundaries remain stable for existing dashboards.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.db.opentelemetry")
                .with_attributes([KeyValue::new("crate", "db")])
                .build(),
        );
        register_db_metrics!(&meter)
    }
}

static METRICS: LazyLock<DbMetrics> = LazyLock::new(DbMetrics::register);

/// Returns the lazily initialized global database metric set.
/// Initialization occurs on first access and is shared process-wide.
pub fn metrics() -> &'static DbMetrics {
    &METRICS
}
