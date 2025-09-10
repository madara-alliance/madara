use anyhow::Context as _;
use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::global::Error;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, KeyValue};
use rocksdb::perf::MemoryUsageBuilder;

use crate::rocksdb::column::ALL_COLUMNS;
use crate::rocksdb::RocksDBStorage;
#[derive(Clone, Debug)]
pub struct DbMetrics {
    pub db_size: Gauge<u64>,
    pub column_sizes: Gauge<u64>,
    pub mem_table_total: Gauge<u64>,
    pub mem_table_unflushed: Gauge<u64>,
    pub mem_table_readers_total: Gauge<u64>,
    pub cache_total: Gauge<u64>,
}

impl DbMetrics {
    pub fn register() -> Result<Self, Error> {
        tracing::trace!("Registering DB metrics.");

        let common_scope_attributes = vec![KeyValue::new("crate", "rpc")];
        let rpc_meter = global::meter_with_version(
            "crates.rpc.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let db_size = register_gauge_metric_instrument(
            &rpc_meter,
            "db_size".to_string(),
            "Node storage usage in GB".to_string(),
            "".to_string(),
        );

        let column_sizes = register_gauge_metric_instrument(
            &rpc_meter,
            "column_sizes".to_string(),
            "Sizes of RocksDB columns".to_string(),
            "".to_string(),
        );

        let mem_table_total = register_gauge_metric_instrument(
            &rpc_meter,
            "db_mem_table_total".to_string(),
            "Approximate memory usage of all the mem-tables in bytes".to_string(),
            "".to_string(),
        );

        let mem_table_unflushed = register_gauge_metric_instrument(
            &rpc_meter,
            "db_mem_table_unflushed".to_string(),
            "Approximate memory usage of un-flushed mem-tables in bytes".to_string(),
            "".to_string(),
        );

        let mem_table_readers_total = register_gauge_metric_instrument(
            &rpc_meter,
            "db_mem_table_readers_total".to_string(),
            "Approximate memory usage of all the table readers in bytes".to_string(),
            "".to_string(),
        );

        let cache_total = register_gauge_metric_instrument(
            &rpc_meter,
            "db_cache_total".to_string(),
            "Approximate memory usage by cache in bytes".to_string(),
            "".to_string(),
        );

        Ok(Self { db_size, column_sizes, mem_table_total, mem_table_unflushed, mem_table_readers_total, cache_total })
    }

    pub fn try_update(&self, db: &RocksDBStorage) -> anyhow::Result<u64> {
        let mut storage_size = 0;

        for column in ALL_COLUMNS {
            let cf_handle = db.inner.get_column(column.clone());
            let cf_metadata = db.inner.db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;

            self.column_sizes.record(column_size, &[KeyValue::new("column", column.rocksdb_name)]);
        }

        self.db_size.record(storage_size, &[]);

        let mut builder = MemoryUsageBuilder::new().context("Creating memory usage builder")?;
        builder.add_db(&db.inner.db);
        let mem_usage = builder.build().context("Getting memory usage")?;
        self.mem_table_total.record(mem_usage.approximate_mem_table_total(), &[]);
        self.mem_table_unflushed.record(mem_usage.approximate_mem_table_unflushed(), &[]);
        self.mem_table_readers_total.record(mem_usage.approximate_mem_table_readers_total(), &[]);
        self.cache_total.record(mem_usage.approximate_cache_total(), &[]);

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
