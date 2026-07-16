use mc_telemetry::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
    register_histogram_metric_instrument_with_boundaries,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
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

    // Reorg context for distinguishing retention pruning from prefix removals
    // performed while Bonsai rolls a trie back.
    pub bonsai_trie_revert_operations: Counter<u64>,
    pub bonsai_trie_revert_revisions: Histogram<u64>,
    pub bonsai_trie_revert_duration: Histogram<f64>,
}

impl DbMetrics {
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.db.opentelemetry")
                .with_attributes([KeyValue::new("crate", "db")])
                .build(),
        );

        // Merklization timing (Priority 1)
        let apply_to_global_trie_duration = register_histogram_metric_instrument(
            &meter,
            "apply_to_global_trie_duration_seconds".to_string(),
            "Total time for global trie merklization".to_string(),
            "s".to_string(),
        );
        let contract_trie_root_duration = register_histogram_metric_instrument(
            &meter,
            "contract_trie_root_duration_seconds".to_string(),
            "Time to compute contract trie root".to_string(),
            "s".to_string(),
        );
        let class_trie_root_duration = register_histogram_metric_instrument(
            &meter,
            "class_trie_root_duration_seconds".to_string(),
            "Time to compute class trie root".to_string(),
            "s".to_string(),
        );
        let contract_storage_trie_commit_duration = register_histogram_metric_instrument(
            &meter,
            "contract_storage_trie_commit_duration_seconds".to_string(),
            "Time to commit contract storage trie".to_string(),
            "s".to_string(),
        );
        let contract_trie_commit_duration = register_histogram_metric_instrument(
            &meter,
            "contract_trie_commit_duration_seconds".to_string(),
            "Time to commit contract trie".to_string(),
            "s".to_string(),
        );
        let class_trie_commit_duration = register_histogram_metric_instrument(
            &meter,
            "class_trie_commit_duration_seconds".to_string(),
            "Time to commit class trie".to_string(),
            "s".to_string(),
        );

        // Block hash calculation (Priority 2)
        let block_commitments_compute_duration = register_histogram_metric_instrument(
            &meter,
            "block_commitments_compute_duration_seconds".to_string(),
            "Total time to compute block commitments".to_string(),
            "s".to_string(),
        );
        let block_hash_compute_duration = register_histogram_metric_instrument(
            &meter,
            "block_hash_compute_duration_seconds".to_string(),
            "Time to compute block hash".to_string(),
            "s".to_string(),
        );

        // Data fetching (Priority 3)
        let get_full_block_without_state_diff_duration = register_histogram_metric_instrument(
            &meter,
            "get_full_block_without_state_diff_duration_seconds".to_string(),
            "Time to fetch full block without state diff".to_string(),
            "s".to_string(),
        );
        let db_write_block_parts_duration = register_histogram_metric_instrument(
            &meter,
            "db_write_block_parts_duration_seconds".to_string(),
            "Time to write block parts to database".to_string(),
            "s".to_string(),
        );

        // Gauges for exact per-block values - Main 5 sequential operations
        let get_full_block_without_state_diff_last = register_gauge_metric_instrument(
            &meter,
            "get_full_block_without_state_diff_last_seconds".to_string(),
            "Last block: time to fetch full block without state diff".to_string(),
            "s".to_string(),
        );
        let block_commitments_compute_last = register_gauge_metric_instrument(
            &meter,
            "block_commitments_compute_last_seconds".to_string(),
            "Last block: time to compute block commitments".to_string(),
            "s".to_string(),
        );
        let apply_to_global_trie_last = register_gauge_metric_instrument(
            &meter,
            "apply_to_global_trie_last_seconds".to_string(),
            "Last block: total time for global trie merklization".to_string(),
            "s".to_string(),
        );
        let block_hash_compute_last = register_gauge_metric_instrument(
            &meter,
            "block_hash_compute_last_seconds".to_string(),
            "Last block: time to compute block hash".to_string(),
            "s".to_string(),
        );
        let db_write_block_parts_last = register_gauge_metric_instrument(
            &meter,
            "db_write_block_parts_last_seconds".to_string(),
            "Last block: time to write block parts to database".to_string(),
            "s".to_string(),
        );

        // Gauges for Merklization Deep Dive
        let contract_trie_root_last = register_gauge_metric_instrument(
            &meter,
            "contract_trie_root_last_seconds".to_string(),
            "Last block: time to compute contract trie root".to_string(),
            "s".to_string(),
        );
        let class_trie_root_last = register_gauge_metric_instrument(
            &meter,
            "class_trie_root_last_seconds".to_string(),
            "Last block: time to compute class trie root".to_string(),
            "s".to_string(),
        );
        let contract_storage_trie_commit_last = register_gauge_metric_instrument(
            &meter,
            "contract_storage_trie_commit_last_seconds".to_string(),
            "Last block: time to commit contract storage trie".to_string(),
            "s".to_string(),
        );
        let contract_trie_commit_last = register_gauge_metric_instrument(
            &meter,
            "contract_trie_commit_last_seconds".to_string(),
            "Last block: time to commit contract trie".to_string(),
            "s".to_string(),
        );
        let class_trie_commit_last = register_gauge_metric_instrument(
            &meter,
            "class_trie_commit_last_seconds".to_string(),
            "Last block: time to commit class trie".to_string(),
            "s".to_string(),
        );

        let bonsai_prefix_prune_operations = register_counter_metric_instrument(
            &meter,
            "bonsai_prefix_prune_operations".to_string(),
            "Number of Bonsai trie-log prefix removals".to_string(),
            "".to_string(),
        );
        let bonsai_prefix_prune_entries_scanned = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_prefix_prune_entries_scanned".to_string(),
            "RocksDB iterator entries examined by each Bonsai trie-log prefix removal".to_string(),
            "".to_string(),
            BONSAI_COUNT_BUCKETS.to_vec(),
        );
        let bonsai_prefix_prune_batch_operations = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_prefix_prune_batch_operations".to_string(),
            "RocksDB write-batch operations generated by each Bonsai trie-log prefix removal".to_string(),
            "".to_string(),
            BONSAI_COUNT_BUCKETS.to_vec(),
        );
        let bonsai_prefix_prune_batch_bytes = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_prefix_prune_batch_bytes".to_string(),
            "Serialized RocksDB write-batch size generated by each Bonsai trie-log prefix removal".to_string(),
            "".to_string(),
            BONSAI_BATCH_BYTES_BUCKETS.to_vec(),
        );
        let bonsai_prefix_prune_duration = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_prefix_prune_duration_seconds".to_string(),
            "Time spent scanning and deleting one Bonsai trie-log prefix".to_string(),
            "s".to_string(),
            BONSAI_DURATION_BUCKETS.to_vec(),
        );
        let bonsai_prefix_prune_last_revision = register_gauge_metric_instrument(
            &meter,
            "bonsai_prefix_prune_last_revision".to_string(),
            "Latest Bonsai trie-log revision fully removed by prefix".to_string(),
            "".to_string(),
        );

        let bonsai_trie_revert_operations = register_counter_metric_instrument(
            &meter,
            "bonsai_trie_revert_operations".to_string(),
            "Number of Bonsai trie revert attempts".to_string(),
            "".to_string(),
        );
        let bonsai_trie_revert_revisions = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_trie_revert_revisions".to_string(),
            "Number of revisions traversed by a Bonsai trie revert attempt".to_string(),
            "".to_string(),
            BONSAI_COUNT_BUCKETS.to_vec(),
        );
        let bonsai_trie_revert_duration = register_histogram_metric_instrument_with_boundaries(
            &meter,
            "bonsai_trie_revert_duration_seconds".to_string(),
            "Time spent reverting one Bonsai trie".to_string(),
            "s".to_string(),
            BONSAI_DURATION_BUCKETS.to_vec(),
        );

        Self {
            apply_to_global_trie_duration,
            contract_trie_root_duration,
            class_trie_root_duration,
            contract_storage_trie_commit_duration,
            contract_trie_commit_duration,
            class_trie_commit_duration,
            block_commitments_compute_duration,
            block_hash_compute_duration,
            get_full_block_without_state_diff_duration,
            db_write_block_parts_duration,
            // Gauges
            get_full_block_without_state_diff_last,
            block_commitments_compute_last,
            apply_to_global_trie_last,
            block_hash_compute_last,
            db_write_block_parts_last,
            contract_trie_root_last,
            class_trie_root_last,
            contract_storage_trie_commit_last,
            contract_trie_commit_last,
            class_trie_commit_last,
            bonsai_prefix_prune_operations,
            bonsai_prefix_prune_entries_scanned,
            bonsai_prefix_prune_batch_operations,
            bonsai_prefix_prune_batch_bytes,
            bonsai_prefix_prune_duration,
            bonsai_prefix_prune_last_revision,
            bonsai_trie_revert_operations,
            bonsai_trie_revert_revisions,
            bonsai_trie_revert_duration,
        }
    }
}

static METRICS: LazyLock<DbMetrics> = LazyLock::new(DbMetrics::register);

/// Get the global database metrics instance
pub fn metrics() -> &'static DbMetrics {
    &METRICS
}
