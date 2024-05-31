use num_traits::FromPrimitive;
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

pub async fn update_metrics(block_metrics: &BlockMetrics, block_header: mp_block::Header) {
    block_metrics.l2_block_number.set(block_header.block_number as f64);
    block_metrics.transaction_count.set(f64::from_u128(block_header.transaction_count).unwrap_or(f64::MIN));
    block_metrics.event_count.set(f64::from_u128(block_header.event_count).unwrap_or(f64::MIN));

    if let Some(l1_gas_price) = block_header.l1_gas_price {
        block_metrics.l1_gas_price_wei.set(f64::from_u128(l1_gas_price.eth_l1_gas_price.into()).unwrap_or(f64::MIN));
        block_metrics.l1_gas_price_strk.set(f64::from_u128(l1_gas_price.strk_l1_gas_price.into()).unwrap_or(f64::MIN));
    }
}
