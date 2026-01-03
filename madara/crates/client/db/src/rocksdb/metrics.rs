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
//! - `db_l0_files_count`: Number of L0 files (configurable via CLI)
//! - `db_num_immutable_memtables`: Memtables waiting to be flushed
//!
//! ## Alerting Thresholds
//!
//! | Metric | Warning | Critical |
//! |--------|---------|----------|
//! | `db_is_write_stopped` | - | = 1 |
//! | `db_pending_compaction_bytes` | > 4 GiB | > 6 GiB |
//! | `db_l0_files_count` | >= 15 | >= 20 |
//! | `db_num_immutable_memtables` | >= 3 | >= 4 |

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
    pub l0_files_count: Gauge<u64>,
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

        let l0_files_count = register_gauge_metric_instrument(
            &meter,
            "db_l0_files_count".to_string(),
            "Number of files at Level 0".to_string(),
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
            l0_files_count,
            num_immutable_memtables,
            memtable_size_bytes,
        })
    }

    pub fn try_update(&self, db: &RocksDBStorage) -> anyhow::Result<u64> {
        let mut storage_size = 0;

        // ═══════════════════════════════════════════════════════════════════════════
        // STORAGE METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        for column in ALL_COLUMNS {
            let cf_handle = db.inner.get_column(column.clone());
            let cf_metadata = db.inner.db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;

            self.column_sizes.record(column_size, &[KeyValue::new("column", column.rocksdb_name)]);
        }

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

        // Number of immutable memtables waiting to be flushed
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.num-immutable-mem-table") {
            self.num_immutable_memtables.record(val, &[]);
        }

        // Total size of all memtables
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.cur-size-all-mem-tables") {
            self.memtable_size_bytes.record(val, &[]);
        }

        // ═══════════════════════════════════════════════════════════════════════════
        // WRITE STALL DETECTION METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        // Is write stopped? (0 = running, 1 = stopped)
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.is-write-stopped") {
            self.is_write_stopped.record(val, &[]);
        }

        // Pending compaction bytes (leading indicator for stalls)
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.estimate-pending-compaction-bytes") {
            self.pending_compaction_bytes.record(val, &[]);
        }

        // L0 file count
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.num-files-at-level0") {
            self.l0_files_count.record(val, &[]);
        }

        Ok(storage_size)
    }

    /// Returns the total storage size
    pub fn update(&self, db: &RocksDBStorage) -> u64 {
        match self.try_update(db) {
            Ok(res) => res,
            Err(err) => {
                tracing::warn!("Error updating db metrics: {err:#}");
                0
            }
        }
    }
}
