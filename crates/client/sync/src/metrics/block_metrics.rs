use prometheus_endpoint::prometheus::Gauge;
use prometheus_endpoint::{register, PrometheusError, Registry};

#[derive(Clone, Debug)]
pub struct BlockMetrics {
    // L2 network metrics
    pub l2_block_number: Gauge,
    pub l2_sync_time: Gauge,
    pub l2_avg_sync_time: Gauge,
    pub l2_latest_sync_time: Gauge,
    pub transaction_count: Gauge,
    pub event_count: Gauge,
    // L1 network metrics
    pub l1_block_number: Gauge,
    pub l1_gas_price_wei: Gauge,
    pub l1_gas_price_strk: Gauge,
}

impl BlockMetrics {
    pub fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        Ok(Self {
            l2_block_number: register(
                Gauge::new("deoxys_l2_block_number", "Gauge for deoxys L2 block number")?,
                registry,
            )?,
            l2_sync_time: register(Gauge::new("deoxys_l2_sync_time", "Gauge for deoxys L2 sync time")?, registry)?,
            l2_avg_sync_time: register(
                Gauge::new("deoxys_l2_avg_sync_time", "Gauge for deoxys L2 average sync time")?,
                registry,
            )?,
            l2_latest_sync_time: register(
                Gauge::new("deoxys_l2_latest_sync_time", "Gauge for deoxys L2 latest sync time")?,
                registry,
            )?,
            l1_block_number: register(
                Gauge::new("deoxys_l1_block_number", "Gauge for deoxys L1 block number")?,
                registry,
            )?,
            transaction_count: register(
                Gauge::new("deoxys_transaction_count", "Gauge for deoxys transaction count")?,
                registry,
            )?,
            event_count: register(Gauge::new("deoxys_event_count", "Gauge for deoxys event count")?, registry)?,
            l1_gas_price_wei: register(Gauge::new("deoxys_l1_gas_price", "Gauge for deoxys L1 gas price")?, registry)?,
            l1_gas_price_strk: register(
                Gauge::new("deoxys_l1_gas_price_strk", "Gauge for deoxys L1 gas price in strk")?,
                registry,
            )?,
        })
    }
}
