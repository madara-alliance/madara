use crate::{Column, DatabaseExt, DB};
use mc_analytics::register_gauge_metric_instrument;
use opentelemetry::global::Error;
use opentelemetry::metrics::Gauge;
use opentelemetry::{global, KeyValue};
#[derive(Clone, Debug)]
pub struct DbMetrics {
    pub db_size: Gauge<u64>,
    pub column_sizes: Gauge<u64>,
}

impl DbMetrics {
    pub fn register() -> Result<Self, Error> {
        tracing::trace!("Registering DB metrics.");
        // TODO: Remove this println
        println!("Registering DB metrics.");

        let common_scope_attributes = vec![KeyValue::new("crate", "rpc")];
        let rpc_meter = global::meter_with_version(
            "crates.rpc.opentelemetry",
            // TODO: Unsure of these settings, come back
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        //  convert this to use otel instead of prometheus
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

        Ok(Self { db_size, column_sizes })
    }

    /// Returns the total storage size
    pub fn update(&self, db: &DB) -> u64 {
        let mut storage_size = 0;

        for &column in Column::ALL.iter() {
            let cf_handle = db.get_column(column);
            let cf_metadata = db.get_column_family_metadata_cf(&cf_handle);
            let column_size = cf_metadata.size;
            storage_size += column_size;

            self.column_sizes.record(column_size, &[KeyValue::new("column", column.rocksdb_name())]);
        }

        storage_size
    }
}
