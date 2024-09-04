use mc_metrics::{IntGaugeVec, MetricsRegistry, Opts, PrometheusError};

#[derive(Clone, Debug)]
pub struct DbMetrics {
    pub column_sizes: IntGaugeVec,
}

impl DbMetrics {
    pub fn register(registry: &MetricsRegistry) -> Result<Self, PrometheusError> {
        Ok(Self {
            column_sizes: registry
                .register(IntGaugeVec::new(Opts::new("column_sizes", "Sizes of RocksDB columns"), &["column"])?)?,
        })
    }
}
