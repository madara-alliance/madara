use mc_analytics::register_gauge_metric_instrument;
use mc_db::MadaraBackend;
use mp_block::Header;
use num_traits::FromPrimitive;
use opentelemetry::metrics::Gauge;
use opentelemetry::{
    global::{self, Error},
    KeyValue,
};
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct BlockMetrics {
    /// Starting block
    pub starting_block: u64,
    pub starting_time: Instant,
    pub last_update_instant: Mutex<Option<Instant>>,
    pub last_db_metrics_update_instant: Mutex<Option<Instant>>,

    // L2 network metrics
    pub l2_block_number: Gauge<u64>,
    pub l2_sync_time: Gauge<f64>,
    pub l2_avg_sync_time: Gauge<f64>,
    pub l2_latest_sync_time: Gauge<f64>,
    pub l2_state_size: Gauge<f64>, // TODO: remove this, as well as the return value from db_metrics update.
    pub transaction_count: Gauge<u64>,
    pub event_count: Gauge<u64>,
    // L1 network metrics
    pub l1_gas_price_wei: Gauge<f64>,
    pub l1_gas_price_strk: Gauge<f64>,
}

impl BlockMetrics {
    pub fn register(starting_block: u64) -> Result<Self, Error> {
        let common_scope_attributes = vec![KeyValue::new("crate", "block_import")];
        let block_import_meter = global::meter_with_version(
            "crates.block_import.opentelemetry",
            Some("0.17"),
            Some("https://opentelemetry.io/schemas/1.2.0"),
            Some(common_scope_attributes.clone()),
        );

        let l2_block_number = register_gauge_metric_instrument(
            &block_import_meter,
            "l2_block_number".to_string(),
            "Current block number".to_string(),
            "".to_string(),
        );

        let l2_sync_time = register_gauge_metric_instrument(
            &block_import_meter,
            "l2_sync_time".to_string(),
            "Complete sync time since startup in secs (does not account for restarts)".to_string(),
            "".to_string(),
        );

        let l2_avg_sync_time = register_gauge_metric_instrument(
            &block_import_meter,
            "l2_avg_sync_time".to_string(),
            "Average time spent between blocks since startup in secs".to_string(),
            "".to_string(),
        );

        let l2_latest_sync_time = register_gauge_metric_instrument(
            &block_import_meter,
            "l2_latest_sync_time".to_string(),
            "Latest time spent between blocks in secs".to_string(),
            "".to_string(),
        );

        let l2_state_size = register_gauge_metric_instrument(
            &block_import_meter,
            "l2_state_size".to_string(),
            "Node storage usage in GB".to_string(),
            "".to_string(),
        );

        let transaction_count = register_gauge_metric_instrument(
            &block_import_meter,
            "transaction_count".to_string(),
            "Latest block transaction count".to_string(),
            "".to_string(),
        );

        let event_count = register_gauge_metric_instrument(
            &block_import_meter,
            "event_count".to_string(),
            "Latest block event count".to_string(),
            "".to_string(),
        );

        let l1_gas_price_wei = register_gauge_metric_instrument(
            &block_import_meter,
            "l1_gas_price_wei".to_string(),
            "Latest block L1 ETH gas price".to_string(),
            "".to_string(),
        );

        let l1_gas_price_strk = register_gauge_metric_instrument(
            &block_import_meter,
            "l1_gas_price_strk".to_string(),
            "Latest block L1 STRK gas price".to_string(),
            "".to_string(),
        );

        Ok(Self {
            starting_block,
            starting_time: Instant::now(),
            last_update_instant: Default::default(),
            last_db_metrics_update_instant: Default::default(),

            l2_block_number,
            l2_sync_time,
            l2_avg_sync_time,
            l2_latest_sync_time,
            l2_state_size,

            transaction_count,
            event_count,

            l1_gas_price_wei,
            l1_gas_price_strk,
        })
    }

    pub fn update(&self, block_header: &Header, backend: &MadaraBackend) {
        let now = Instant::now();

        // Update Block sync time metrics
        let latest_sync_time = {
            let mut last_update = self.last_update_instant.lock().expect("Poisoned lock");
            let latest_sync_time = last_update.map(|inst| now.duration_since(inst)).unwrap_or_default();
            *last_update = Some(now);
            latest_sync_time.as_secs_f64()
        };

        let total_sync_time = now.duration_since(self.starting_time).as_secs_f64();

        self.l2_sync_time.record(total_sync_time, &[]);
        self.l2_latest_sync_time.record(latest_sync_time, &[]);
        self.l2_avg_sync_time.record(total_sync_time / (block_header.block_number - self.starting_block) as f64, &[]);

        self.l2_block_number.record(block_header.block_number, &[]);
        self.transaction_count.record(block_header.transaction_count, &[]);
        self.event_count.record(block_header.event_count, &[]);

        self.l1_gas_price_wei.record(f64::from_u128(block_header.l1_gas_price.eth_l1_gas_price).unwrap_or(0f64), &[]);
        self.l1_gas_price_strk.record(f64::from_u128(block_header.l1_gas_price.strk_l1_gas_price).unwrap_or(0f64), &[]);

        {
            let mut last_db_instant = self.last_db_metrics_update_instant.lock().expect("Poisoned lock");
            let last_update_duration = last_db_instant.map(|inst| now.duration_since(inst));

            if last_update_duration.is_none() || last_update_duration.is_some_and(|d| d >= Duration::from_secs(5)) {
                *last_db_instant = Some(now);
                let storage_size = backend.update_metrics();
                let size_gb = storage_size as f64 / (1024 * 1024 * 1024) as f64;
                self.l2_state_size.record(size_gb, &[]);
            }
        }
    }
}
