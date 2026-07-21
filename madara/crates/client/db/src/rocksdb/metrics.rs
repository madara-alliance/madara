//! RocksDB metrics for monitoring database health, compaction throughput, and write stalls.
//!
//! ## Capacity Planning Metrics
//!
//! The cumulative ticker metrics below require `MADARA_DB_ENABLE_STATISTICS=true`.
//! RocksDB property metrics, including the per-column compaction and memtable
//! gauges, are available without statistics. Key queries for Grafana:
//! - Ingest rate: `rate(db_bytes_written[5m])`
//! - Compaction throughput: `rate(db_compact_write_bytes[5m])`
//! - Write amplification: `rate(db_compact_write_bytes[5m]) / rate(db_flush_write_bytes[5m])`
//! - Stall fraction: `rate(db_stall_micros[5m]) / 1e6`
//! - Cache hit rate: `rate(db_block_cache_hit[5m]) / (rate(db_block_cache_hit[5m]) + rate(db_block_cache_miss[5m]))`
//!
//! ## Write Stall Detection
//!
//! | Metric | Meaning |
//! |--------|---------|
//! | `db_is_write_stopped` | RocksDB has stopped accepting writes |
//! | `db_pending_compaction_bytes` | Estimated compaction work per CF and as `column="total"` |
//! | `db_column_level_files_count` | Per-CF SST count at each LSM level |
//! | `db_level_files_count` | Aggregate SST count across all CFs at each LSM level |
//! | `db_column_memtable_deletes` | Point-delete pressure in Bonsai log memtables |
//! | `db_num_snapshots` | Snapshots that can retain old RocksDB versions |
//! | `db_stall_micros` rate > 0 | Time spent in write stalls |
//!
//! ## Note on `pending_compaction_bytes`
//!
//! `rocksdb.estimate-pending-compaction-bytes` always returns 0 for column families using
//! universal compaction. Only leveled compaction CFs (the `bonsai_*_log` CFs) report
//! meaningful values. Alert rules must select either a specific `column` or
//! `column="total"`; an unfiltered query mixes per-CF and synthetic-total series.
//! Alert thresholds are monitoring policy and are independent from RocksDB's
//! configured soft and hard write limits.

use crate::rocksdb::column::ALL_COLUMNS;
use crate::rocksdb::RocksDBStorage;
use anyhow::Context;
use mc_telemetry::register_gauge_metric_instrument;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, InstrumentationScope, KeyValue};
use rocksdb::perf::MemoryUsageBuilder;
use rocksdb::statistics::Ticker;
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub column_num_immutable_memtables: Gauge<u64>,
    pub column_memtable_size_bytes: Gauge<u64>,
    pub column_memtable_entries: Gauge<u64>,
    pub column_memtable_deletes: Gauge<u64>,

    // Write stall detection metrics
    pub is_write_stopped: Gauge<u64>,
    pub pending_compaction_bytes: Gauge<u64>,
    pub column_compaction_pending: Gauge<u64>,
    pub column_memtable_flush_pending: Gauge<u64>,

    // LSM tree level metrics
    pub level_files_count: Gauge<u64>,
    pub column_level_files_count: Gauge<u64>,
    pub column_estimated_keys: Gauge<u64>,
    pub column_estimated_live_data_size_bytes: Gauge<u64>,
    pub column_live_versions: Gauge<u64>,

    // Compaction & I/O throughput gauges (cumulative values from RocksDB Statistics tickers).
    // These are monotonically increasing; use rate() in Grafana to get per-second throughput.
    pub bytes_written: Gauge<u64>,
    pub flush_write_bytes: Gauge<u64>,
    pub compact_read_bytes: Gauge<u64>,
    pub compact_write_bytes: Gauge<u64>,
    pub keys_written: Gauge<u64>,
    pub keys_updated: Gauge<u64>,
    pub stall_micros: Gauge<u64>,
    pub block_cache_hit: Gauge<u64>,
    pub block_cache_miss: Gauge<u64>,

    // Compaction activity gauges
    pub running_compactions: Gauge<u64>,
    pub running_flushes: Gauge<u64>,
    pub actual_delayed_write_rate: Gauge<u64>,
    pub background_errors: Gauge<u64>,
    pub num_snapshots: Gauge<u64>,
    pub oldest_snapshot_age_seconds: Gauge<u64>,
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

        let column_num_immutable_memtables = register_gauge_metric_instrument(
            &meter,
            "db_column_num_immutable_memtables".to_string(),
            "Number of immutable memtables waiting to be flushed in each column family".to_string(),
            "".to_string(),
        );

        let column_memtable_size_bytes = register_gauge_metric_instrument(
            &meter,
            "db_column_memtable_size_bytes".to_string(),
            "Size of active and unflushed immutable memtables in each column family".to_string(),
            "".to_string(),
        );

        let column_memtable_entries = register_gauge_metric_instrument(
            &meter,
            "db_column_memtable_entries".to_string(),
            "Number of entries in Bonsai log column-family memtables".to_string(),
            "".to_string(),
        );

        let column_memtable_deletes = register_gauge_metric_instrument(
            &meter,
            "db_column_memtable_deletes".to_string(),
            "Number of delete entries in Bonsai log column-family memtables".to_string(),
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

        let column_compaction_pending = register_gauge_metric_instrument(
            &meter,
            "db_column_compaction_pending".to_string(),
            "Whether a compaction is pending for a Bonsai log column family (0=no, 1=yes)".to_string(),
            "".to_string(),
        );

        let column_memtable_flush_pending = register_gauge_metric_instrument(
            &meter,
            "db_column_memtable_flush_pending".to_string(),
            "Whether a memtable flush is pending for a Bonsai log column family (0=no, 1=yes)".to_string(),
            "".to_string(),
        );

        let column_level_files_count = register_gauge_metric_instrument(
            &meter,
            "db_column_level_files_count".to_string(),
            "Number of SST files at each LSM level in each column family".to_string(),
            "".to_string(),
        );

        let column_estimated_keys = register_gauge_metric_instrument(
            &meter,
            "db_column_estimated_keys".to_string(),
            "Estimated number of keys in each Bonsai log column family".to_string(),
            "".to_string(),
        );

        let column_estimated_live_data_size_bytes = register_gauge_metric_instrument(
            &meter,
            "db_column_estimated_live_data_size_bytes".to_string(),
            "Estimated live-data size in each Bonsai log column family".to_string(),
            "".to_string(),
        );

        let column_live_versions = register_gauge_metric_instrument(
            &meter,
            "db_column_live_versions".to_string(),
            "Number of live RocksDB versions held for each Bonsai log column family".to_string(),
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

        let keys_written = register_gauge_metric_instrument(
            &meter,
            "db_keys_written".to_string(),
            "Cumulative keys written through RocksDB write batches".to_string(),
            "".to_string(),
        );

        let keys_updated = register_gauge_metric_instrument(
            &meter,
            "db_keys_updated".to_string(),
            "Cumulative existing keys updated by RocksDB".to_string(),
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

        let background_errors = register_gauge_metric_instrument(
            &meter,
            "db_background_errors".to_string(),
            "Cumulative RocksDB background errors".to_string(),
            "".to_string(),
        );

        let num_snapshots = register_gauge_metric_instrument(
            &meter,
            "db_num_snapshots".to_string(),
            "Number of unreleased RocksDB snapshots".to_string(),
            "".to_string(),
        );

        let oldest_snapshot_age_seconds = register_gauge_metric_instrument(
            &meter,
            "db_oldest_snapshot_age_seconds".to_string(),
            "Age of the oldest unreleased RocksDB snapshot".to_string(),
            "s".to_string(),
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
            column_num_immutable_memtables,
            column_memtable_size_bytes,
            column_memtable_entries,
            column_memtable_deletes,
            column_compaction_pending,
            column_memtable_flush_pending,
            column_level_files_count,
            column_estimated_keys,
            column_estimated_live_data_size_bytes,
            column_live_versions,
            bytes_written,
            flush_write_bytes,
            compact_read_bytes,
            compact_write_bytes,
            keys_written,
            keys_updated,
            stall_micros,
            block_cache_hit,
            block_cache_miss,
            running_compactions,
            running_flushes,
            actual_delayed_write_rate,
            background_errors,
            num_snapshots,
            oldest_snapshot_age_seconds,
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
            let column_attribute = [KeyValue::new("column", column.rocksdb_name)];

            let cf_metadata = db.inner.db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;
            self.column_sizes.record(column_size, &column_attribute);

            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.num-immutable-mem-table") {
                total_immutable_memtables += val;
                self.column_num_immutable_memtables.record(val, &column_attribute);
            }

            if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.cur-size-all-mem-tables") {
                total_memtable_size += val;
                self.column_memtable_size_bytes.record(val, &column_attribute);
            }

            // pending_compaction_bytes: report per-CF (universal CFs always report 0)
            if let Ok(Some(val)) =
                db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.estimate-pending-compaction-bytes")
            {
                total_pending_compaction += val;
                self.pending_compaction_bytes.record(val, &column_attribute);
            }

            for (level, count) in total_files_at_level.iter_mut().enumerate() {
                let property = format!("rocksdb.num-files-at-level{}", level);
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, &property) {
                    *count += val;
                    self.column_level_files_count.record(
                        val,
                        &[KeyValue::new("column", column.rocksdb_name), KeyValue::new("level", format!("L{level}"))],
                    );
                }
            }

            // Detailed tombstone and compaction-pressure metrics are limited to
            // the three leveled Bonsai log CFs. These are the only CFs where
            // pending-compaction bytes are meaningful and where prefix pruning
            // writes one point tombstone per retained trie-log entry.
            if column.log_cf {
                for (state, entries_property, deletes_property) in [
                    ("active", "rocksdb.num-entries-active-mem-table", "rocksdb.num-deletes-active-mem-table"),
                    ("immutable", "rocksdb.num-entries-imm-mem-tables", "rocksdb.num-deletes-imm-mem-tables"),
                ] {
                    let attributes = [KeyValue::new("column", column.rocksdb_name), KeyValue::new("state", state)];
                    if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, entries_property) {
                        self.column_memtable_entries.record(val, &attributes);
                    }
                    if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, deletes_property) {
                        self.column_memtable_deletes.record(val, &attributes);
                    }
                }

                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.compaction-pending") {
                    self.column_compaction_pending.record(val, &column_attribute);
                }
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.mem-table-flush-pending")
                {
                    self.column_memtable_flush_pending.record(val, &column_attribute);
                }
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.estimate-num-keys") {
                    self.column_estimated_keys.record(val, &column_attribute);
                }
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.estimate-live-data-size")
                {
                    self.column_estimated_live_data_size_bytes.record(val, &column_attribute);
                }
                if let Ok(Some(val)) = db.inner.db.property_int_value_cf(&cf_handle, "rocksdb.num-live-versions") {
                    self.column_live_versions.record(val, &column_attribute);
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

        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.background-errors") {
            self.background_errors.record(val, &[]);
        }
        if let Ok(Some(val)) = db.inner.db.property_int_value("rocksdb.num-snapshots") {
            self.num_snapshots.record(val, &[]);
        }
        if let Ok(Some(oldest_snapshot_time)) = db.inner.db.property_int_value("rocksdb.oldest-snapshot-time") {
            let age = if oldest_snapshot_time == 0 {
                0
            } else {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                now.saturating_sub(oldest_snapshot_time)
            };
            self.oldest_snapshot_age_seconds.record(age, &[]);
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
        self.keys_written.record(opts.get_ticker_count(Ticker::NumberKeysWritten), &[]);
        self.keys_updated.record(opts.get_ticker_count(Ticker::NumberKeysUpdated), &[]);
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
    use crate::rocksdb::{RocksDBConfig, RocksDBStorage};
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
        let keys_written = stored.global_opts.get_ticker_count(Ticker::NumberKeysWritten);
        println!("BytesWritten:      {bytes_written}");
        println!("FlushWriteBytes:   {}", stored.global_opts.get_ticker_count(Ticker::FlushWriteBytes));
        println!("CompactReadBytes:  {}", stored.global_opts.get_ticker_count(Ticker::CompactReadBytes));
        println!("CompactWriteBytes: {}", stored.global_opts.get_ticker_count(Ticker::CompactWriteBytes));
        println!("BlockCacheHit:     {}", stored.global_opts.get_ticker_count(Ticker::BlockCacheHit));
        println!("BlockCacheMiss:    {}", stored.global_opts.get_ticker_count(Ticker::BlockCacheMiss));
        println!("StallMicros:       {}", stored.global_opts.get_ticker_count(Ticker::StallMicros));

        assert!(bytes_written > 0, "BytesWritten should be > 0 after 1000 puts, got {bytes_written}");
        assert!(keys_written > 0, "NumberKeysWritten should be > 0 after 1000 puts, got {keys_written}");
    }

    #[test]
    fn compaction_metrics_bonsai_columns_expose_required_properties() {
        let directory = tempfile::tempdir().unwrap();
        let storage = RocksDBStorage::open(directory.path(), RocksDBConfig::default()).unwrap();
        let metrics = super::DbMetrics::register().unwrap();

        metrics.try_update(&storage).unwrap();

        for property in ["rocksdb.background-errors", "rocksdb.num-snapshots", "rocksdb.oldest-snapshot-time"] {
            assert!(
                storage.inner.db.property_int_value(property).unwrap().is_some(),
                "property {property} should be available"
            );
        }

        let bonsai_log_columns: Vec<_> = ALL_COLUMNS.iter().filter(|column| column.log_cf).collect();
        assert_eq!(bonsai_log_columns.len(), 3);

        for column in bonsai_log_columns {
            let handle = storage.inner.get_column(column.clone());
            for property in [
                "rocksdb.num-entries-active-mem-table",
                "rocksdb.num-entries-imm-mem-tables",
                "rocksdb.num-deletes-active-mem-table",
                "rocksdb.num-deletes-imm-mem-tables",
                "rocksdb.compaction-pending",
                "rocksdb.mem-table-flush-pending",
                "rocksdb.estimate-num-keys",
                "rocksdb.estimate-live-data-size",
                "rocksdb.num-live-versions",
            ] {
                assert!(
                    storage.inner.db.property_int_value_cf(&handle, property).unwrap().is_some(),
                    "property {property} should be available for {}",
                    column.rocksdb_name
                );
            }
        }
    }
}
