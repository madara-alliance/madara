use prometheus_endpoint::prometheus::Gauge;
use prometheus_endpoint::{register, PrometheusError, Registry};

#[derive(Clone, Debug)]
pub struct BlockMetrics {
    pub l2_block_number: Gauge,
    pub l1_block_number: Gauge,
    pub transaction_count: Gauge,
    pub event_count: Gauge,
    pub l1_gas_price_wei: Gauge,
    pub l1_gas_price_strk: Gauge,
}

impl BlockMetrics {
    pub fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        Ok(Self {
            l2_block_number: register(Gauge::new("deoxys_l2_block_number", "Gauge for deoxys L2 block number")?, registry)?,
            l1_block_number: register(Gauge::new("deoxys_l1_block_number", "Gauge for deoxys L1 block number")?, registry)?,
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
