use prometheus_endpoint::prometheus::{Counter, Gauge};
use prometheus_endpoint::{register, PrometheusError, Registry};

#[derive(Clone, Debug)]
pub struct BlockMetrics {
    pub block_height: Gauge,
    pub transaction_count: Counter,
    pub event_count: Counter,
    pub l1_gas_price_wei: Gauge,
    pub l1_gas_price_strk: Gauge,
}

impl BlockMetrics {
    pub fn register(registry: &Registry) -> Result<Self, PrometheusError> {
        Ok(Self {
            block_height: register(Gauge::new("deoxys_block_height", "Gauge for deoxys block height")?, registry)?,
            transaction_count: register(
                Counter::new("deoxys_transaction_count", "Counter for deoxys transaction count")?,
                registry,
            )?,
            event_count: register(Counter::new("deoxys_event_count", "Counter for deoxys event count")?, registry)?,
            l1_gas_price_wei: register(Gauge::new("deoxys_l1_gas_price", "Gauge for deoxys l1 gas price")?, registry)?,
            l1_gas_price_strk: register(
                Gauge::new("deoxys_l1_gas_price_strk", "Gauge for deoxys l1 gas price in strk")?,
                registry,
            )?,
        })
    }
}
