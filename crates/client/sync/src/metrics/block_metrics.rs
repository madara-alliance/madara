use mc_metrics::{Gauge, MetricsRegistry, PrometheusError, F64};

#[derive(Clone, Debug)]
pub struct BlockMetrics {
    // L2 network metrics
    pub l2_block_number: Gauge<F64>,
    pub l2_sync_time: Gauge<F64>,
    pub l2_avg_sync_time: Gauge<F64>,
    pub l2_latest_sync_time: Gauge<F64>,
    pub l2_state_size: Gauge<F64>,
    pub transaction_count: Gauge<F64>,
    pub event_count: Gauge<F64>,
    // L1 network metrics
    // gas price is also define in eth/client.rs but this would be the gas used in the block and it's price
    pub l1_gas_price_wei: Gauge<F64>,
    pub l1_gas_price_strk: Gauge<F64>,
}

impl BlockMetrics {
    pub fn register(registry: &MetricsRegistry) -> Result<Self, PrometheusError> {
        Ok(Self {
            l2_block_number: registry
                .register(Gauge::new("madara_l2_block_number", "Gauge for madara L2 block number")?)?,
            l2_sync_time: registry.register(Gauge::new("madara_l2_sync_time", "Gauge for madara L2 sync time")?)?,
            l2_avg_sync_time: registry
                .register(Gauge::new("madara_l2_avg_sync_time", "Gauge for madara L2 average sync time")?)?,
            l2_latest_sync_time: registry
                .register(Gauge::new("madara_l2_latest_sync_time", "Gauge for madara L2 latest sync time")?)?,
            l2_state_size: registry
                .register(Gauge::new("madara_l2_state_size", "Gauge for node storage usage in GB")?)?,
            transaction_count: registry
                .register(Gauge::new("madara_transaction_count", "Gauge for madara transaction count")?)?,
            event_count: registry.register(Gauge::new("madara_event_count", "Gauge for madara event count")?)?,
            l1_gas_price_wei: registry
                .register(Gauge::new("madara_l1_block_gas_price", "Gauge for madara L1 gas price")?)?,
            l1_gas_price_strk: registry
                .register(Gauge::new("madara_l1_block_gas_price_strk", "Gauge for madara L1 gas price in strk")?)?,
        })
    }
}
