use mc_analytics::{register_gauge_metric_instrument, register_histogram_metric_instrument};
use opentelemetry::metrics::{Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use std::sync::LazyLock;

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
    pub get_full_block_with_classes_duration: Histogram<f64>,
    pub db_write_block_parts_duration: Histogram<f64>,

    // Gauges for exact per-block values - Main 5 sequential operations
    pub get_full_block_with_classes_last: Gauge<f64>,
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
        let get_full_block_with_classes_duration = register_histogram_metric_instrument(
            &meter,
            "get_full_block_with_classes_duration_seconds".to_string(),
            "Time to fetch full block with classes".to_string(),
            "s".to_string(),
        );
        let db_write_block_parts_duration = register_histogram_metric_instrument(
            &meter,
            "db_write_block_parts_duration_seconds".to_string(),
            "Time to write block parts to database".to_string(),
            "s".to_string(),
        );

        // Gauges for exact per-block values - Main 5 sequential operations
        let get_full_block_with_classes_last = register_gauge_metric_instrument(
            &meter,
            "get_full_block_with_classes_last_seconds".to_string(),
            "Last block: time to fetch full block with classes".to_string(),
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

        Self {
            apply_to_global_trie_duration,
            contract_trie_root_duration,
            class_trie_root_duration,
            contract_storage_trie_commit_duration,
            contract_trie_commit_duration,
            class_trie_commit_duration,
            block_commitments_compute_duration,
            block_hash_compute_duration,
            get_full_block_with_classes_duration,
            db_write_block_parts_duration,
            // Gauges
            get_full_block_with_classes_last,
            block_commitments_compute_last,
            apply_to_global_trie_last,
            block_hash_compute_last,
            db_write_block_parts_last,
            contract_trie_root_last,
            class_trie_root_last,
            contract_storage_trie_commit_last,
            contract_trie_commit_last,
            class_trie_commit_last,
        }
    }
}

static METRICS: LazyLock<DbMetrics> = LazyLock::new(DbMetrics::register);

/// Get the global database metrics instance
pub fn metrics() -> &'static DbMetrics {
    &METRICS
}
