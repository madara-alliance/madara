use crate::{Column, DatabaseExt, DB};
use mc_metrics::{Gauge, IntGaugeVec, MetricsRegistry, Opts, PrometheusError, F64};

#[derive(Clone, Debug)]
pub struct DbMetrics {
    pub db_size: Gauge<F64>,
    pub column_sizes: IntGaugeVec,
}

impl DbMetrics {
    pub fn register(registry: &MetricsRegistry) -> Result<Self, PrometheusError> {
        Ok(Self {
            db_size: registry.register(Gauge::new("db_size", "Node storage usage in GB")?)?,
            column_sizes: registry
                .register(IntGaugeVec::new(Opts::new("column_sizes", "Sizes of RocksDB columns"), &["column"])?)?,
        })
    }

    /// Returns the total storage size
    pub fn update(&self, db: &DB) -> u64 {
        let mut storage_size = 0;

        for &column in Column::ALL.iter() {
            let cf_handle = db.get_column(column);
            let cf_metadata = db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;

            self.column_sizes.with_label_values(&[column.rocksdb_name()]).set(column_size as i64);
        }

        storage_size
    }
}
