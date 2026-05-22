//! RocksDB metrics for monitoring database health, compaction throughput, and write stalls.
//!
//! ## Capacity Planning Metrics (requires statistics enabled, which is the default)
//!
//! Key queries for Grafana:
//! - Ingest rate: `rate(db_bytes_written[5m])`
//! - Compaction throughput: `rate(db_compact_write_bytes[5m])`
//! - Write amplification: `rate(db_compact_write_bytes[5m]) / rate(db_flush_write_bytes[5m])`
//! - Stall fraction: `rate(db_stall_micros[5m]) / 1e6`
//! - Cache hit rate: `rate(db_block_cache_hit[5m]) / (rate(db_block_cache_hit[5m]) + rate(db_block_cache_miss[5m]))`
//!
//! ## Write Stall Detection
//!
//! | Metric | Warning | Critical |
//! |--------|---------|----------|
//! | `db_is_write_stopped` | - | = 1 |
//! | `db_pending_compaction_bytes` | > 4 GiB | > 6 GiB |
//! | `db_level_files_count` | L0: >= 15 | L0: >= 20 |
//! | `db_stall_micros` rate > 0 | warning | - |
//!
//! ## Note on `pending_compaction_bytes`
//!
//! `rocksdb.estimate-pending-compaction-bytes` always returns 0 for column families using
//! universal compaction. Only leveled compaction CFs (the `bonsai_*_log` CFs) report
//! meaningful values. The per-CF label lets you filter to leveled CFs only.

use crate::rocksdb::column::ALL_COLUMNS;
use crate::rocksdb::RocksDBStorage;
use anyhow::Context;
use mc_telemetry::register_gauge_metric_instrument;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use rocksdb::perf::MemoryUsageBuilder;
use rocksdb::statistics::Ticker;

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

    // Compaction & I/O throughput gauges (cumulative values from RocksDB Statistics tickers).
    // These are monotonically increasing; use rate() in Grafana to get per-second throughput.
    pub bytes_written: Gauge<u64>,
    pub flush_write_bytes: Gauge<u64>,
    pub compact_read_bytes: Gauge<u64>,
    pub compact_write_bytes: Gauge<u64>,
    pub stall_micros: Gauge<u64>,
    pub block_cache_hit: Gauge<u64>,
    pub block_cache_miss: Gauge<u64>,

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
        // LSM TREE & MEMTABLE METRICS
        // ═══════════════════════════════════════════════════════════════════════════

        let level_files_count = register_gauge_metric_instrument(
            &meter,
            "db_level_files_count".to_string(),
            "Number of SST files at each LSM tree level".to_string(),
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
            "Estimated bytes pending compaction (always 0 for universal compaction CFs)".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // COMPACTION & I/O THROUGHPUT (cumulative ticker values, requires statistics)
        // ═══════════════════════════════════════════════════════════════════════════

        let bytes_written = register_gauge_metric_instrument(
            &meter,
            "db_bytes_written".to_string(),
            "Cumulative bytes written by application (Put/Merge/Delete)".to_string(),
            "".to_string(),
        );

        let flush_write_bytes = register_gauge_metric_instrument(
            &meter,
            "db_flush_write_bytes".to_string(),
            "Cumulative bytes written by memtable flushes to L0".to_string(),
            "".to_string(),
        );

        let compact_read_bytes = register_gauge_metric_instrument(
            &meter,
            "db_compact_read_bytes".to_string(),
            "Cumulative bytes read during compaction".to_string(),
            "".to_string(),
        );

        let compact_write_bytes = register_gauge_metric_instrument(
            &meter,
            "db_compact_write_bytes".to_string(),
            "Cumulative bytes written during compaction".to_string(),
            "".to_string(),
        );

        let stall_micros = register_gauge_metric_instrument(
            &meter,
            "db_stall_micros".to_string(),
            "Cumulative microseconds spent in write stalls".to_string(),
            "".to_string(),
        );

        let block_cache_hit = register_gauge_metric_instrument(
            &meter,
            "db_block_cache_hit".to_string(),
            "Cumulative block cache hits".to_string(),
            "".to_string(),
        );

        let block_cache_miss = register_gauge_metric_instrument(
            &meter,
            "db_block_cache_miss".to_string(),
            "Cumulative block cache misses (each miss = a disk read)".to_string(),
            "".to_string(),
        );

        // ═══════════════════════════════════════════════════════════════════════════
        // COMPACTION ACTIVITY
        // ═══════════════════════════════════════════════════════════════════════════

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
            running_compactions,
            running_flushes,
            actual_delayed_write_rate,
        })
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

        // Global property (not per-CF): 1 if writes are completely blocked
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.is-write-stopped") {
            self.is_write_stopped.record(val, &[]);
        }

        self.pending_compaction_bytes.record(total_pending_compaction, &[KeyValue::new("column", "total")]);

        // ═══════════════════════════════════════════════════════════════════════════
        // COMPACTION & I/O THROUGHPUT (from Statistics tickers, DB-wide)
        // ═══════════════════════════════════════════════════════════════════════════

        let opts = &db.inner.global_opts;
        self.bytes_written.record(opts.get_ticker_count(Ticker::BytesWritten), &[]);
        self.flush_write_bytes.record(opts.get_ticker_count(Ticker::FlushWriteBytes), &[]);
        self.compact_read_bytes.record(opts.get_ticker_count(Ticker::CompactReadBytes), &[]);
        self.compact_write_bytes.record(opts.get_ticker_count(Ticker::CompactWriteBytes), &[]);
        self.stall_micros.record(opts.get_ticker_count(Ticker::StallMicros), &[]);
        self.block_cache_hit.record(opts.get_ticker_count(Ticker::BlockCacheHit), &[]);
        self.block_cache_miss.record(opts.get_ticker_count(Ticker::BlockCacheMiss), &[]);

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

#[cfg(test)]
mod tests {
    use crate::rocksdb::column::ALL_COLUMNS;
    use crate::rocksdb::options::rocksdb_global_options;
    use crate::rocksdb::RocksDBConfig;
    use rocksdb::statistics::Ticker;
    use rocksdb::{ColumnFamilyDescriptor, DBWithThreadMode, MultiThreaded, Options as RocksDBOptions};
    use std::sync::Arc;

    struct StoredOpts {
        db: DBWithThreadMode<MultiThreaded>,
        global_opts: RocksDBOptions,
    }

    #[test]
    fn test_ticker_via_stored_opts_after_cf_descriptors_open() {
        let dir = tempfile::tempdir().unwrap();
        let config = RocksDBConfig::default();
        let opts = rocksdb_global_options(&config).unwrap();

        let cfs: Vec<ColumnFamilyDescriptor> = ALL_COLUMNS
            .iter()
            .map(|col| ColumnFamilyDescriptor::new(col.rocksdb_name, col.rocksdb_options(&config)))
            .collect();

        let db = DBWithThreadMode::<MultiThreaded>::open_cf_descriptors(&opts, dir.path(), cfs).unwrap();

        let stored = Arc::new(StoredOpts { db, global_opts: opts });

        let writeopts = config.write_mode.to_write_options();
        let cf = stored.db.cf_handle("block_info").unwrap();
        for i in 0..1000u32 {
            stored.db.put_cf_opt(&cf, i.to_be_bytes(), vec![0u8; 256], &writeopts).unwrap();
        }

        let bytes_written = stored.global_opts.get_ticker_count(Ticker::BytesWritten);
        println!("BytesWritten:      {bytes_written}");
        println!("FlushWriteBytes:   {}", stored.global_opts.get_ticker_count(Ticker::FlushWriteBytes));
        println!("CompactReadBytes:  {}", stored.global_opts.get_ticker_count(Ticker::CompactReadBytes));
        println!("CompactWriteBytes: {}", stored.global_opts.get_ticker_count(Ticker::CompactWriteBytes));
        println!("BlockCacheHit:     {}", stored.global_opts.get_ticker_count(Ticker::BlockCacheHit));
        println!("BlockCacheMiss:    {}", stored.global_opts.get_ticker_count(Ticker::BlockCacheMiss));
        println!("StallMicros:       {}", stored.global_opts.get_ticker_count(Ticker::StallMicros));

        assert!(bytes_written > 0, "BytesWritten should be > 0 after 1000 puts, got {bytes_written}");
    }
}
