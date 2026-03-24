use crate::counter::ThroughputCounter;
use anyhow::Context;
use mc_analytics::{register_counter_metric_instrument, register_histogram_metric_instrument};
use mc_db::{MadaraBackend, MadaraStorageRead};
use num_traits::cast::FromPrimitive;
use opentelemetry::{
    global,
    metrics::{Counter, Histogram},
    InstrumentationScope, KeyValue,
};
use std::{
    sync::Arc,
    sync::LazyLock,
    time::{Duration, Instant},
};

pub struct SyncMetrics {
    /// Built-in throughput counter, for logging purposes
    pub counter: ThroughputCounter,

    /// Starting block
    pub starting_block: u64,
    pub starting_time: Instant,
    pub last_update_instant: Option<Instant>,
    pub last_db_metrics_update_instant: Option<Instant>,

    // L2 network metrics
    pub l2_block_number: Histogram<f64>,
    pub l2_sync_time: Histogram<f64>,
    pub l2_avg_sync_time: Histogram<f64>,
    pub l2_latest_sync_time: Histogram<f64>,
    pub transaction_count: Counter<u64>,
    pub event_count: Counter<u64>,
    // L1 network metrics
    // gas price is also define in eth/client.rs but this would be the gas used in the block and it's price
    pub l1_gas_price_wei: Histogram<f64>,
    pub l1_gas_price_strk: Histogram<f64>,
}

pub struct SyncControlMetrics {
    pub reorg_detected_total: Counter<u64>,
    pub reorg_processed_total: Counter<u64>,
    pub reorg_required_but_disabled_total: Counter<u64>,
    pub genesis_mismatch_total: Counter<u64>,
    pub genesis_recovery_total: Counter<u64>,
    pub pipeline_reinitialized_total: Counter<u64>,
    pub common_ancestor_search_duration: Histogram<f64>,
    pub common_ancestor_distance_blocks: Histogram<f64>,
    pub genesis_recovery_duration: Histogram<f64>,
}

impl SyncMetrics {
    pub fn register(starting_block: u64) -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.sync.opentelemetry")
                .with_attributes([KeyValue::new("crate", "sync")])
                .build(),
        );

        let l2_block_number = register_histogram_metric_instrument(
            &meter,
            "l2_block_number".to_string(),
            "Gauge for madara L2 block number".to_string(),
            "".to_string(),
        );

        let l2_sync_time = register_histogram_metric_instrument(
            &meter,
            "l2_sync_time".to_string(),
            "Gauge for madara L2 sync time".to_string(),
            "".to_string(),
        );

        let l2_avg_sync_time = register_histogram_metric_instrument(
            &meter,
            "l2_avg_sync_time".to_string(),
            "Gauge for madara L2 average sync time".to_string(),
            "".to_string(),
        );

        let l2_latest_sync_time = register_histogram_metric_instrument(
            &meter,
            "l2_latest_sync_time".to_string(),
            "Gauge for madara L2 latest sync time".to_string(),
            "".to_string(),
        );

        let transaction_count = register_counter_metric_instrument(
            &meter,
            "transaction_count".to_string(),
            "Gauge for madara transaction count".to_string(),
            "".to_string(),
        );

        let event_count = register_counter_metric_instrument(
            &meter,
            "event_count".to_string(),
            "Gauge for madara event count".to_string(),
            "".to_string(),
        );

        let l1_gas_price_wei = register_histogram_metric_instrument(
            &meter,
            "l1_gas_price_wei".to_string(),
            "Gauge for madara L1 gas price in wei".to_string(),
            "".to_string(),
        );

        let l1_gas_price_strk = register_histogram_metric_instrument(
            &meter,
            "l1_gas_price_strk".to_string(),
            "Gauge for madara L1 gas price in strk".to_string(),
            "".to_string(),
        );

        Self {
            counter: ThroughputCounter::new(Duration::from_secs(5 * 60)),

            starting_block,
            starting_time: Instant::now(),
            last_update_instant: Default::default(),
            last_db_metrics_update_instant: Default::default(),

            l2_block_number,
            l2_sync_time,
            l2_avg_sync_time,
            l2_latest_sync_time,

            transaction_count,
            event_count,

            l1_gas_price_wei,
            l1_gas_price_strk,
        }
    }

    pub fn update(&mut self, block_n: u64, backend: &Arc<MadaraBackend>) -> anyhow::Result<()> {
        let now = Instant::now();

        // Update Block sync time metrics
        let latest_sync_time = self.last_update_instant.map(|inst| now.duration_since(inst)).unwrap_or_default();
        let latest_sync_time = latest_sync_time.as_secs_f64();
        self.last_update_instant = Some(now);

        self.counter.increment();

        let header = backend
            .db
            .get_block_info(block_n)? // Raw get
            .context("No block info")?
            .header;

        let total_sync_time = now.duration_since(self.starting_time).as_secs_f64();

        self.l2_sync_time.record(total_sync_time, &[]);
        self.l2_latest_sync_time.record(latest_sync_time, &[]);
        self.l2_avg_sync_time.record(total_sync_time / (header.block_number - self.starting_block) as f64, &[]);

        self.l2_block_number.record(header.block_number as _, &[]);
        self.transaction_count.add(header.transaction_count, &[]);
        self.event_count.add(header.event_count, &[]);

        self.l1_gas_price_wei.record(f64::from_u128(header.gas_prices.eth_l1_gas_price).unwrap_or(0f64), &[]);
        self.l1_gas_price_strk.record(f64::from_u128(header.gas_prices.strk_l1_gas_price).unwrap_or(0f64), &[]);

        Ok(())
    }
}

impl SyncControlMetrics {
    fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.sync.control.opentelemetry")
                .with_attributes([KeyValue::new("crate", "sync")])
                .build(),
        );

        Self {
            reorg_detected_total: register_counter_metric_instrument(
                &meter,
                "sync_reorg_detected_total".to_string(),
                "Number of upstream reorgs detected during sync".to_string(),
                "reorg".to_string(),
            ),
            reorg_processed_total: register_counter_metric_instrument(
                &meter,
                "sync_reorg_processed_total".to_string(),
                "Number of reorgs successfully processed during sync".to_string(),
                "reorg".to_string(),
            ),
            reorg_required_but_disabled_total: register_counter_metric_instrument(
                &meter,
                "sync_reorg_required_but_disabled_total".to_string(),
                "Number of reorgs detected while auto-reorg handling was disabled".to_string(),
                "reorg".to_string(),
            ),
            genesis_mismatch_total: register_counter_metric_instrument(
                &meter,
                "sync_genesis_mismatch_total".to_string(),
                "Number of genesis mismatches detected during sync".to_string(),
                "mismatch".to_string(),
            ),
            genesis_recovery_total: register_counter_metric_instrument(
                &meter,
                "sync_genesis_recovery_total".to_string(),
                "Number of destructive genesis recovery runs triggered during sync".to_string(),
                "recovery".to_string(),
            ),
            pipeline_reinitialized_total: register_counter_metric_instrument(
                &meter,
                "sync_pipeline_reinitialized_total".to_string(),
                "Number of sync pipeline reinitializations after recovery events".to_string(),
                "reinit".to_string(),
            ),
            common_ancestor_search_duration: register_histogram_metric_instrument(
                &meter,
                "sync_common_ancestor_search_duration_seconds".to_string(),
                "Time spent searching for the common ancestor during reorg handling".to_string(),
                "s".to_string(),
            ),
            common_ancestor_distance_blocks: register_histogram_metric_instrument(
                &meter,
                "sync_common_ancestor_distance_blocks".to_string(),
                "Number of blocks walked backwards while searching for the common ancestor".to_string(),
                "block".to_string(),
            ),
            genesis_recovery_duration: register_histogram_metric_instrument(
                &meter,
                "sync_genesis_recovery_duration_seconds".to_string(),
                "Time spent wiping and refreshing local state after a genesis mismatch".to_string(),
                "s".to_string(),
            ),
        }
    }
}

static CONTROL_METRICS: LazyLock<SyncControlMetrics> = LazyLock::new(SyncControlMetrics::register);

pub fn control_metrics() -> &'static SyncControlMetrics {
    &CONTROL_METRICS
}
