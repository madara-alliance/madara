use crate::{Column, DatabaseExt, DB};
use anyhow::Context;
use mc_metrics::{prometheus::IntGauge, IntGaugeVec, MetricsRegistry, Opts, PrometheusError};
use rocksdb::perf::MemoryUsageBuilder;

#[derive(Clone, Debug)]
pub struct DbMetrics {
    pub size: IntGauge,
    pub column_sizes: IntGaugeVec,
    pub mem_table_total: IntGauge,
    pub mem_table_unflushed: IntGauge,
    pub mem_table_readers_total: IntGauge,
    pub cache_total: IntGauge,
}

impl DbMetrics {
    pub fn register(registry: &MetricsRegistry) -> Result<Self, PrometheusError> {
        Ok(Self {
            size: registry.register(IntGauge::new("db_size", "Node storage usage in bytes")?)?,
            column_sizes: registry
                .register(IntGaugeVec::new(Opts::new("db_column_sizes", "Sizes of RocksDB columns"), &["column"])?)?,
            mem_table_total: registry.register(IntGauge::new(
                "db_mem_table_total",
                "Approximate memory usage of all the mem-tables in bytes",
            )?)?,
            mem_table_unflushed: registry.register(IntGauge::new(
                "db_mem_table_unflushed",
                "Approximate memory usage of un-flushed mem-tables in bytes",
            )?)?,
            mem_table_readers_total: registry.register(IntGauge::new(
                "db_mem_table_readers_total",
                "Approximate memory usage of all the table readers in bytes",
            )?)?,
            cache_total: registry
                .register(IntGauge::new("db_cache_total", "Approximate memory usage by cache in bytes")?)?,
        })
    }

    pub fn try_update(&self, db: &DB) -> anyhow::Result<u64> {
        let mut storage_size = 0;

        for &column in Column::ALL.iter() {
            let cf_handle = db.get_column(column);
            let cf_metadata = db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;

            self.column_sizes.with_label_values(&[column.rocksdb_name()]).set(column_size as i64);
        }

        self.size.set(storage_size as _);

        let mut builder = MemoryUsageBuilder::new().context("Creating memory usage builder")?;
        builder.add_db(db);
        let mem_usage = builder.build().context("Getting memory usage")?;
        self.mem_table_total.set(mem_usage.approximate_mem_table_total() as _);
        self.mem_table_unflushed.set(mem_usage.approximate_mem_table_unflushed() as _);
        self.mem_table_readers_total.set(mem_usage.approximate_mem_table_readers_total() as _);
        self.cache_total.set(mem_usage.approximate_cache_total() as _);

        Ok(storage_size)
    }

    /// Returns the total storage size
    pub fn update(&self, db: &DB) -> u64 {
        match self.try_update(db) {
            Ok(res) => res,
            Err(err) => {
                log::warn!("Error updating db metrics: {err:#}");
                0
            }
        }
    }
}
