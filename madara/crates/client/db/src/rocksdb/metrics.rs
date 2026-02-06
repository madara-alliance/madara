//! RocksDB metrics for monitoring database health and detecting write stalls.
//!
//! These metrics are exported via OpenTelemetry and can be scraped by Prometheus
//! or pushed to an OTLP endpoint.
//!
//! ## Write Stall Detection
//!
//! Key metrics for detecting write stalls:
//! - `db_is_write_stopped`: 1 if writes are completely blocked
//! - `db_pending_compaction_bytes`: Backlog of data waiting to be compacted
//! - `db_level_files_count`: Number of files at each LSM tree level
//! - `db_num_immutable_memtables`: Memtables waiting to be flushed
//!
//! ## Alerting Thresholds
//!
//! | Metric | Warning | Critical |
//! |--------|---------|----------|
//! | `db_is_write_stopped` | - | = 1 |
//! | `db_pending_compaction_bytes` | > 4 GiB | > 6 GiB |
//! | `db_level_files_count` | L0: >= 15 | L0: >= 20 |
//! | `db_num_immutable_memtables` | >= 3 | >= 4 |
//!
//! ## Contract Cache Metrics
//!
//! Metrics for monitoring the contract-specific cache:
//! - `contract_cache_hits`: Number of cache hits
//! - `contract_cache_misses`: Number of cache misses
//! - `contract_cache_evictions`: Number of LRU evictions
//! - `contract_cache_memory_bytes`: Current memory usage
//! - `contract_cache_storage_entries`: Number of cached storage entries
//! - `contract_cache_class_entries`: Number of cached class entries

use crate::rocksdb::column::ALL_COLUMNS;
use crate::rocksdb::RocksDBStorage;
use anyhow::Context;
use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use rocksdb::perf::MemoryUsageBuilder;

#[derive(Clone, Debug)]
pub struct DbMetrics {
    // Storage metrics
    pub db_size: Gauge<u64>,
    pub column_sizes: Gauge<u64>,

    // Memory metrics
    pub mem_table_total: Gauge<u64>,
    pub mem_table_unflushed: Gauge<u64>,
    pub mem_table_readers_total: Gauge<u64>,
    pub cache_total: Gauge<u64>,
    pub num_immutable_memtables: Gauge<u64>,
    pub memtable_size_bytes: Gauge<u64>,

    // Write stall detection metrics
    pub is_write_stopped: Gauge<u64>,
    pub pending_compaction_bytes: Gauge<u64>,

    // LSM tree level metrics
    pub level_files_count: Gauge<u64>,

    // Contract cache metrics
    pub contract_cache_hits: Gauge<u64>,
    pub contract_cache_misses: Gauge<u64>,
    pub contract_cache_evictions: Gauge<u64>,
    pub contract_cache_memory_bytes: Gauge<u64>,
    pub contract_cache_storage_entries: Gauge<u64>,
    pub contract_cache_class_entries: Gauge<u64>,
}

impl DbMetrics {
    pub fn register() -> anyhow::Result<Self> {
        tracing::trace!("Registering DB metrics.");

        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.db.opentelemetry")
                .with_attributes([KeyValue::new("crate", "db")])
                .build(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // STORAGE METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let db_size = register_gauge_metric_instrument(
            &meter,
            "db_size".to_string(),
            "Total database storage size in bytes".to_string(),
            "".to_string(),
        );

        let column_sizes = register_gauge_metric_instrument(
            &meter,
            "column_sizes".to_string(),
            "Size of each RocksDB column family in bytes".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // MEMORY METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let mem_table_total = register_gauge_metric_instrument(
            &meter,
            "db_mem_table_total".to_string(),
            "Approximate memory usage of all memtables in bytes".to_string(),
            "".to_string(),
        );

        let mem_table_unflushed = register_gauge_metric_instrument(
            &meter,
            "db_mem_table_unflushed".to_string(),
            "Approximate memory usage of unflushed memtables in bytes".to_string(),
            "".to_string(),
        );

        let mem_table_readers_total = register_gauge_metric_instrument(
            &meter,
            "db_mem_table_readers_total".to_string(),
            "Approximate memory usage of all table readers in bytes".to_string(),
            "".to_string(),
        );

        let cache_total = register_gauge_metric_instrument(
            &meter,
            "db_cache_total".to_string(),
            "Approximate memory usage by block cache in bytes".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // LSM TREE LEVEL METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let level_files_count = register_gauge_metric_instrument(
            &meter,
            "db_level_files_count".to_string(),
            "Number of SST files at each LSM tree level".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // MEMTABLE MONITORING
        // ═══════════════════════════════════════════════════════════════════════════

        let num_immutable_memtables = register_gauge_metric_instrument(
            &meter,
            "db_num_immutable_memtables".to_string(),
            "Number of immutable memtables waiting to be flushed (stall when >= max_write_buffer_number)".to_string(),
            "".to_string(),
        );

        let memtable_size_bytes = register_gauge_metric_instrument(
            &meter,
            "db_memtable_size_bytes".to_string(),
            "Total size of all memtables in bytes".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // WRITE STALL DETECTION METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let is_write_stopped = register_gauge_metric_instrument(
            &meter,
            "db_is_write_stopped".to_string(),
            "Whether RocksDB has stopped accepting writes (0=running, 1=stopped)".to_string(),
            "".to_string(),
        );

        let pending_compaction_bytes = register_gauge_metric_instrument(
            &meter,
            "db_pending_compaction_bytes".to_string(),
            "Estimated bytes pending compaction".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // CONTRACT CACHE METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let contract_cache_hits = register_gauge_metric_instrument(
            &meter,
            "contract_cache_hits".to_string(),
            "Number of contract cache hits".to_string(),
            "".to_string(),
        );

        let contract_cache_misses = register_gauge_metric_instrument(
            &meter,
            "contract_cache_misses".to_string(),
            "Number of contract cache misses".to_string(),
            "".to_string(),
        );

        let contract_cache_evictions = register_gauge_metric_instrument(
            &meter,
            "contract_cache_evictions".to_string(),
            "Number of contract cache LRU evictions".to_string(),
            "".to_string(),
        );

        let contract_cache_memory_bytes = register_gauge_metric_instrument(
            &meter,
            "contract_cache_memory_bytes".to_string(),
            "Current contract cache memory usage in bytes".to_string(),
            "".to_string(),
        );

        let contract_cache_storage_entries = register_gauge_metric_instrument(
            &meter,
            "contract_cache_storage_entries".to_string(),
            "Number of cached storage entries".to_string(),
            "".to_string(),
        );

        let contract_cache_class_entries = register_gauge_metric_instrument(
            &meter,
            "contract_cache_class_entries".to_string(),
            "Number of cached class entries".to_string(),
            "".to_string(),
        );

        Ok(Self {
            db_size,
            column_sizes,
            mem_table_total,
            mem_table_unflushed,
            mem_table_readers_total,
            cache_total,
            is_write_stopped,
            pending_compaction_bytes,
            level_files_count,
            num_immutable_memtables,
            memtable_size_bytes,
            contract_cache_hits,
            contract_cache_misses,
            contract_cache_evictions,
            contract_cache_memory_bytes,
            contract_cache_storage_entries,
            contract_cache_class_entries,
        })
    }

    pub fn try_update(&self, db: &RocksDBStorage) -> anyhow::Result<u64> {
        let mut storage_size: u64 = 0;
        let mut total_immutable_memtables: u64 = 0;
        let mut total_memtable_size: u64 = 0;
        let mut total_pending_compaction: u64 = 0;
        let mut total_files_at_level: [u64; 7] = [0; 7];

        // ═══════════════════════════════════════════════════════════════════════════
        // PER-COLUMN-FAMILY METRICS (aggregated across all columns)
        // ═══════════════════════════════════════════════════════════════════════════

        for column in ALL_COLUMNS {
            let cf_handle = db.inner.get_column(column.clone());

            // Storage size
            let cf_metadata = db.inner.db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;
            self.column_sizes.record(column_size, &[KeyValue::new("column", column.rocksdb_name)]);

            // Immutable memtables waiting to be flushed
            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.num-immutable-mem-table") {
                total_immutable_memtables += val;
            }

            // Memtable size
            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.cur-size-all-mem-tables") {
                total_memtable_size += val;
            }

            // Pending compaction bytes
            if let Ok(Some(val)) =
                db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.estimate-pending-compaction-bytes")
            {
                total_pending_compaction += val;
            }

            // Record file counts for levels 0-6 (RocksDB typically uses up to 7 levels)
            for (level, count) in total_files_at_level.iter_mut().enumerate() {
                let property = format!("rocksdb.num-files-at-level{}", level);
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, &property) {
                    *count += val;
                }
            }
        }

        // Record file counts for levels 0-6
        for (level, count) in total_files_at_level.iter().enumerate() {
            self.level_files_count.record(*count, &[KeyValue::new("level", format!("L{}", level))]);
        }

        // Record aggregated storage metrics
        self.db_size.record(storage_size, &[]);

        // ═══════════════════════════════════════════════════════════════════════════
        // MEMORY METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let mut builder = MemoryUsageBuilder::new().context("Creating memory usage builder")?;
        builder.add_db(&db.inner.db);
        let mem_usage = builder.build().context("Getting memory usage")?;
        self.mem_table_total.record(mem_usage.approximate_mem_table_total(), &[]);
        self.mem_table_unflushed.record(mem_usage.approximate_mem_table_unflushed(), &[]);
        self.mem_table_readers_total.record(mem_usage.approximate_mem_table_readers_total(), &[]);
        self.cache_total.record(mem_usage.approximate_cache_total(), &[]);

        // Record aggregated per-CF metrics
        self.num_immutable_memtables.record(total_immutable_memtables, &[]);
        self.memtable_size_bytes.record(total_memtable_size, &[]);

        // ═══════════════════════════════════════════════════════════════════════════
        // WRITE STALL DETECTION METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        // Is write stopped? (global property, not per-CF)
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.is-write-stopped") {
            self.is_write_stopped.record(val, &[]);
        }

        // Record aggregated per-CF metrics
        self.pending_compaction_bytes.record(total_pending_compaction, &[]);

        // ═══════════════════════════════════════════════════════════════════════════
        // CONTRACT CACHE METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        if let Some(cache) = &db.contract_cache {
            self.contract_cache_hits.record(cache.hits(), &[]);
            self.contract_cache_misses.record(cache.misses(), &[]);
            self.contract_cache_evictions.record(cache.evictions(), &[]);
            self.contract_cache_memory_bytes.record(cache.memory_bytes() as u64, &[]);
            self.contract_cache_storage_entries.record(cache.storage_entry_count() as u64, &[]);
            self.contract_cache_class_entries
                .record((cache.class_info_entry_count() + cache.compiled_class_entry_count()) as u64, &[]);
        }

        Ok(storage_size)
    }

    /// Returns the total storage size
    pub fn update(&self, db: &RocksDBStorage) -> u64 {
        self.try_update(db).unwrap_or_else(|err| {
            tracing::warn!("Error updating db metrics: {err:#}");
            0
        })
    }
}
