use mc_db::MadaraBackend;
use mc_metrics::{Gauge, MetricsRegistry, PrometheusError, F64};
use mp_block::Header;
use num_traits::FromPrimitive;
use std::{sync::Mutex, time::Instant};

#[derive(Debug)]
pub struct BlockMetrics {
    /// Starting block
    pub starting_block: u64,
    pub last_block_instant: Mutex<Option<Instant>>,

    // L2 network metrics
    pub l2_block_number: Gauge<F64>,
    pub l2_sync_time: Gauge<F64>,
    pub l2_avg_sync_time: Gauge<F64>,
    pub l2_latest_sync_time: Gauge<F64>,
    pub l2_state_size: Gauge<F64>, // TODO: remove this, as well as the return value from db_metrics update.
    pub transaction_count: Gauge<F64>,
    pub event_count: Gauge<F64>,
    // L1 network metrics
    pub l1_gas_price_wei: Gauge<F64>,
    pub l1_gas_price_strk: Gauge<F64>,
}

impl BlockMetrics {
    pub fn register(starting_block: u64, registry: &MetricsRegistry) -> Result<Self, PrometheusError> {
        Ok(Self {
            starting_block,
            last_block_instant: Default::default(),
            l2_block_number: registry.register(Gauge::new("madara_l2_block_number", "Current block number")?)?,
            l2_sync_time: registry.register(Gauge::new(
                "madara_l2_sync_time",
                "Complete sync time since startup (does not account for restarts)",
            )?)?,
            l2_avg_sync_time: registry
                .register(Gauge::new("madara_l2_avg_sync_time", "Average time spent between blocks since startup")?)?,
            l2_latest_sync_time: registry
                .register(Gauge::new("madara_l2_latest_sync_time", "Latest time spent between blocks")?)?,
            l2_state_size: registry.register(Gauge::new("madara_l2_state_size", "Node storage usage in GB")?)?,
            transaction_count: registry
                .register(Gauge::new("madara_transaction_count", "Latest block transaction count")?)?,
            event_count: registry.register(Gauge::new("madara_event_count", "Latest block event count")?)?,
            l1_gas_price_wei: registry
                .register(Gauge::new("madara_l1_block_gas_price", "Latest block L1 ETH gas price")?)?,
            l1_gas_price_strk: registry
                .register(Gauge::new("madara_l1_block_gas_price_strk", "Latest block L1 STRK gas price")?)?,
        })
    }

    pub fn update(&self, block_header: &Header, backend: &MadaraBackend) {
        // Update Block sync time metrics
        let elapsed_time = {
            let mut timer_guard = self.last_block_instant.lock().expect("Poisoned lock");
            *timer_guard = Some(Instant::now());
            if let Some(start_time) = *timer_guard {
                start_time.elapsed().as_secs_f64()
            } else {
                // For the first block, there is no previous timer set
                0.0
            }
        };

        let sync_time = self.l2_sync_time.get() + elapsed_time;
        self.l2_sync_time.set(sync_time);
        self.l2_latest_sync_time.set(elapsed_time);
        self.l2_avg_sync_time.set(self.l2_sync_time.get() / (block_header.block_number - self.starting_block) as f64);

        self.l2_block_number.set(block_header.block_number as f64);
        self.transaction_count.set(f64::from_u64(block_header.transaction_count).unwrap_or(0f64));
        self.event_count.set(f64::from_u64(block_header.event_count).unwrap_or(0f64));

        self.l1_gas_price_wei.set(f64::from_u128(block_header.l1_gas_price.eth_l1_gas_price).unwrap_or(0f64));
        self.l1_gas_price_strk.set(f64::from_u128(block_header.l1_gas_price.strk_l1_gas_price).unwrap_or(0f64));

        if block_header.block_number % 200 == 0 {
            let storage_size = backend.update_metrics();
            let size_gb = storage_size as f64 / (1024 * 1024 * 1024) as f64;
            self.l2_state_size.set(size_gb);
        }
    }
}
