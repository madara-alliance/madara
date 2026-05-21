//! RocksDB metrics for monitoring database health, compaction throughput, and write stalls.
//!
//! ## Capacity Planning Metrics (requires statistics enabled, which is the default)
//!
//! Key queries for Grafana:
//! - Ingest rate: `rate(db_bytes_written_total[5m])`
//! - Compaction throughput: `rate(db_compact_write_bytes_total[5m])`
//! - Write amplification: `rate(db_compact_write_bytes_total[5m]) / rate(db_flush_write_bytes_total[5m])`
//! - Stall fraction: `rate(db_stall_micros_total[5m]) / 1e6`
//! - Cache hit rate: `rate(db_block_cache_hit_total[5m]) / (rate(db_block_cache_hit_total[5m]) + rate(db_block_cache_miss_total[5m]))`
//!
//! ## Write Stall Detection
//!
//! | Metric | Warning | Critical |
//! |--------|---------|----------|
//! | `db_is_write_stopped` | - | = 1 |
//! | `db_pending_compaction_bytes` | > 4 GiB | > 6 GiB |
//! | `db_level_files_count` | L0: >= 15 | L0: >= 20 |
//! | `db_stall_micros_total` rate > 0 | warning | - |
//!
//! ## Note on `pending_compaction_bytes`
//!
//! `rocksdb.estimate-pending-compaction-bytes` always returns 0 for column families using
//! universal compaction. Only leveled compaction CFs (the `bonsai_*_log` CFs) report
//! meaningful values. The per-CF label lets you filter to leveled CFs only.

use crate::rocksdb::column::ALL_COLUMNS;
use crate::rocksdb::RocksDBStorage;
use anyhow::Context;
use mc_telemetry::{register_counter_metric_instrument, register_gauge_metric_instrument};
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use rocksdb::perf::MemoryUsageBuilder;
use rocksdb::statistics::Ticker;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug)]
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

    // Compaction & I/O throughput counters (from RocksDB Statistics tickers)
    pub bytes_written: Counter<u64>,
    pub flush_write_bytes: Counter<u64>,
    pub compact_read_bytes: Counter<u64>,
    pub compact_write_bytes: Counter<u64>,
    pub stall_micros: Counter<u64>,
    pub block_cache_hit: Counter<u64>,
    pub block_cache_miss: Counter<u64>,

    // Previous ticker values for delta computation (get_ticker_count returns cumulative values)
    prev_bytes_written: AtomicU64,
    prev_flush_write_bytes: AtomicU64,
    prev_compact_read_bytes: AtomicU64,
    prev_compact_write_bytes: AtomicU64,
    prev_stall_micros: AtomicU64,
    prev_block_cache_hit: AtomicU64,
    prev_block_cache_miss: AtomicU64,

    // Compaction activity gauges
    pub running_compactions: Gauge<u64>,
    pub running_flushes: Gauge<u64>,
    pub actual_delayed_write_rate: Gauge<u64>,
}

impl DbMetrics {
    pub fn register() -> anyhow::Result<Self> {
        tracing::trace!("Registering DB metrics.");

        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.db.opentelemetry")
                .with_attributes([KeyValue::new("crate", "db")])
                .build(),
        );

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

        let level_files_count = register_gauge_metric_instrument(
            &meter,
            "db_level_files_count".to_string(),
            "Number of SST files at each LSM tree level".to_string(),
            "".to_string(),
        );

        let num_immutable_memtables = register_gauge_metric_instrument(
            &meter,
            "db_num_immutable_memtables".to_string(),
            "Number of immutable memtables waiting to be flushed".to_string(),
            "".to_string(),
        );

        let memtable_size_bytes = register_gauge_metric_instrument(
            &meter,
            "db_memtable_size_bytes".to_string(),
            "Total size of all memtables in bytes".to_string(),
            "".to_string(),
        );

        let is_write_stopped = register_gauge_metric_instrument(
            &meter,
            "db_is_write_stopped".to_string(),
            "Whether RocksDB has stopped accepting writes (0=running, 1=stopped)".to_string(),
            "".to_string(),
        );

        let pending_compaction_bytes = register_gauge_metric_instrument(
            &meter,
            "db_pending_compaction_bytes".to_string(),
            "Estimated bytes pending compaction (always 0 for universal compaction CFs)".to_string(),
            "".to_string(),
        );

        // Compaction & I/O throughput counters
        let bytes_written = register_counter_metric_instrument(
            &meter,
            "db_bytes_written".to_string(),
            "Cumulative bytes written by application (Put/Merge/Delete)".to_string(),
            "".to_string(),
        );

        let flush_write_bytes = register_counter_metric_instrument(
            &meter,
            "db_flush_write_bytes".to_string(),
            "Cumulative bytes written by memtable flushes to L0".to_string(),
            "".to_string(),
        );

        let compact_read_bytes = register_counter_metric_instrument(
            &meter,
            "db_compact_read_bytes".to_string(),
            "Cumulative bytes read during compaction".to_string(),
            "".to_string(),
        );

        let compact_write_bytes = register_counter_metric_instrument(
            &meter,
            "db_compact_write_bytes".to_string(),
            "Cumulative bytes written during compaction".to_string(),
            "".to_string(),
        );

        let stall_micros = register_counter_metric_instrument(
            &meter,
            "db_stall_micros".to_string(),
            "Cumulative microseconds spent in write stalls".to_string(),
            "".to_string(),
        );

        let block_cache_hit = register_counter_metric_instrument(
            &meter,
            "db_block_cache_hit".to_string(),
            "Cumulative block cache hits".to_string(),
            "".to_string(),
        );

        let block_cache_miss = register_counter_metric_instrument(
            &meter,
            "db_block_cache_miss".to_string(),
            "Cumulative block cache misses (each miss = a disk read)".to_string(),
            "".to_string(),
        );

        // Compaction activity gauges
        let running_compactions = register_gauge_metric_instrument(
            &meter,
            "db_running_compactions".to_string(),
            "Number of currently running compaction jobs".to_string(),
            "".to_string(),
        );

        let running_flushes = register_gauge_metric_instrument(
            &meter,
            "db_running_flushes".to_string(),
            "Number of currently running flush jobs".to_string(),
            "".to_string(),
        );

        let actual_delayed_write_rate = register_gauge_metric_instrument(
            &meter,
            "db_actual_delayed_write_rate".to_string(),
            "Current throttled write rate in bytes/sec when slowdown triggers fire".to_string(),
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
            bytes_written,
            flush_write_bytes,
            compact_read_bytes,
            compact_write_bytes,
            stall_micros,
            block_cache_hit,
            block_cache_miss,
            prev_bytes_written: AtomicU64::new(0),
            prev_flush_write_bytes: AtomicU64::new(0),
            prev_compact_read_bytes: AtomicU64::new(0),
            prev_compact_write_bytes: AtomicU64::new(0),
            prev_stall_micros: AtomicU64::new(0),
            prev_block_cache_hit: AtomicU64::new(0),
            prev_block_cache_miss: AtomicU64::new(0),
            running_compactions,
            running_flushes,
            actual_delayed_write_rate,
        })
    }

    fn add_ticker_delta(&self, counter: &Counter<u64>, prev: &AtomicU64, current: u64) {
        let previous = prev.swap(current, Ordering::Relaxed);
        let delta = current.saturating_sub(previous);
        if delta > 0 {
            counter.add(delta, &[]);
        }
    }

    pub fn try_update(&self, db: &RocksDBStorage) -> anyhow::Result<u64> {
        let mut storage_size: u64 = 0;
        let mut total_immutable_memtables: u64 = 0;
        let mut total_memtable_size: u64 = 0;
        let mut total_pending_compaction: u64 = 0;
        let mut total_files_at_level: [u64; 7] = [0; 7];

        // ═══════════════════════════════════════════════════════════════════════════
        // PER-COLUMN-FAMILY METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        for column in ALL_COLUMNS {
            let cf_handle = db.inner.get_column(column.clone());

            let cf_metadata = db.inner.db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;
            self.column_sizes.record(column_size, &[KeyValue::new("column", column.rocksdb_name)]);

            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.num-immutable-mem-table") {
                total_immutable_memtables += val;
            }

            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.cur-size-all-mem-tables") {
                total_memtable_size += val;
            }

            // pending_compaction_bytes: report per-CF (universal CFs always report 0)
            if let Ok(Some(val)) =
                db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.estimate-pending-compaction-bytes")
            {
                total_pending_compaction += val;
                self.pending_compaction_bytes.record(val, &[KeyValue::new("column", column.rocksdb_name)]);
            }

            for (level, count) in total_files_at_level.iter_mut().enumerate() {
                let property = format!("rocksdb.num-files-at-level{}", level);
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, &property) {
                    *count += val;
                }
            }
        }

        for (level, count) in total_files_at_level.iter().enumerate() {
            self.level_files_count.record(*count, &[KeyValue::new("level", format!("L{}", level))]);
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

        self.num_immutable_memtables.record(total_immutable_memtables, &[]);
        self.memtable_size_bytes.record(total_memtable_size, &[]);

        // ═══════════════════════════════════════════════════════════════════════════
        // WRITE STALL DETECTION
        // ═══════════════════════════════════════════════════════════════════════════

        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.is-write-stopped") {
            self.is_write_stopped.record(val, &[]);
        }

        self.pending_compaction_bytes.record(total_pending_compaction, &[KeyValue::new("column", "total")]);

        // ═══════════════════════════════════════════════════════════════════════════
        // COMPACTION & I/O THROUGHPUT (from Statistics tickers, DB-wide)
        // ═══════════════════════════════════════════════════════════════════════════

        let opts = &db.inner.global_opts;
        self.add_ticker_delta(
            &self.bytes_written,
            &self.prev_bytes_written,
            opts.get_ticker_count(Ticker::BytesWritten),
        );
        self.add_ticker_delta(
            &self.flush_write_bytes,
            &self.prev_flush_write_bytes,
            opts.get_ticker_count(Ticker::FlushWriteBytes),
        );
        self.add_ticker_delta(
            &self.compact_read_bytes,
            &self.prev_compact_read_bytes,
            opts.get_ticker_count(Ticker::CompactReadBytes),
        );
        self.add_ticker_delta(
            &self.compact_write_bytes,
            &self.prev_compact_write_bytes,
            opts.get_ticker_count(Ticker::CompactWriteBytes),
        );
        self.add_ticker_delta(&self.stall_micros, &self.prev_stall_micros, opts.get_ticker_count(Ticker::StallMicros));
        self.add_ticker_delta(
            &self.block_cache_hit,
            &self.prev_block_cache_hit,
            opts.get_ticker_count(Ticker::BlockCacheHit),
        );
        self.add_ticker_delta(
            &self.block_cache_miss,
            &self.prev_block_cache_miss,
            opts.get_ticker_count(Ticker::BlockCacheMiss),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // COMPACTION ACTIVITY (global properties)
        // ═══════════════════════════════════════════════════════════════════════════

        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.num-running-compactions") {
            self.running_compactions.record(val, &[]);
        }
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.num-running-flushes") {
            self.running_flushes.record(val, &[]);
        }
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.actual-delayed-write-rate") {
            self.actual_delayed_write_rate.record(val, &[]);
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
