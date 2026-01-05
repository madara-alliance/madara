use crate::util::ExecutionStats;
use mc_analytics::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};

pub struct BlockProductionMetrics {
    pub block_gauge: Gauge<u64>,
    pub block_counter: Counter<u64>,
    pub transaction_counter: Counter<u64>,
    pub block_production_time: Histogram<f64>,
    // Batch execution stats
    pub batches_executed: Counter<u64>,
    pub txs_added_to_block: Counter<u64>,
    pub txs_executed: Counter<u64>,
    pub txs_reverted: Counter<u64>,
    pub txs_rejected: Counter<u64>,
    pub classes_declared: Counter<u64>,
    pub l2_gas_consumed: Counter<u64>,
}

impl BlockProductionMetrics {
    pub fn register() -> Self {
        // Register meter
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.block_production.opentelemetry")
                .with_attributes([KeyValue::new("crate", "block_production")])
                .build(),
        );

        let block_gauge = register_gauge_metric_instrument(
            &meter,
            "block_produced_no".to_string(),
            "A gauge to show block state at given time".to_string(),
            "block".to_string(),
        );
        let block_counter = register_counter_metric_instrument(
            &meter,
            "block_produced_count".to_string(),
            "A counter to show block state at given time".to_string(),
            "block".to_string(),
        );
        let transaction_counter = register_counter_metric_instrument(
            &meter,
            "transaction_counter".to_string(),
            "A counter to show transaction state for the given block".to_string(),
            "transaction".to_string(),
        );
        let block_production_time = register_histogram_metric_instrument(
            &meter,
            "block_production_time".to_string(),
            "Time to produce a full block from start to close".to_string(),
            "s".to_string(),
        );

        // Batch execution stats
        let batches_executed = register_counter_metric_instrument(
            &meter,
            "batches_executed".to_string(),
            "Number of batches executed during block production".to_string(),
            "batch".to_string(),
        );
        let txs_added_to_block = register_counter_metric_instrument(
            &meter,
            "txs_added_to_block".to_string(),
            "Number of transactions successfully added to blocks".to_string(),
            "tx".to_string(),
        );
        let txs_executed = register_counter_metric_instrument(
            &meter,
            "txs_executed".to_string(),
            "Total number of transactions executed".to_string(),
            "tx".to_string(),
        );
        let txs_reverted = register_counter_metric_instrument(
            &meter,
            "txs_reverted".to_string(),
            "Number of reverted transactions (included but failed)".to_string(),
            "tx".to_string(),
        );
        let txs_rejected = register_counter_metric_instrument(
            &meter,
            "txs_rejected".to_string(),
            "Number of rejected transactions (not included in block)".to_string(),
            "tx".to_string(),
        );
        let classes_declared = register_counter_metric_instrument(
            &meter,
            "classes_declared".to_string(),
            "Number of classes declared".to_string(),
            "class".to_string(),
        );
        let l2_gas_consumed = register_counter_metric_instrument(
            &meter,
            "l2_gas_consumed".to_string(),
            "Total L2 gas consumed by transactions".to_string(),
            "gas".to_string(),
        );

        Self {
            block_gauge,
            block_counter,
            transaction_counter,
            block_production_time,
            batches_executed,
            txs_added_to_block,
            txs_executed,
            txs_reverted,
            txs_rejected,
            classes_declared,
            l2_gas_consumed,
        }
    }

    pub fn record_execution_stats(&self, stats: &ExecutionStats) {
        self.batches_executed.add(stats.n_batches as u64, &[]);
        self.txs_added_to_block.add(stats.n_added_to_block as u64, &[]);
        self.txs_executed.add(stats.n_executed as u64, &[]);
        self.txs_reverted.add(stats.n_reverted as u64, &[]);
        self.txs_rejected.add(stats.n_rejected as u64, &[]);
        self.classes_declared.add(stats.declared_classes as u64, &[]);
        self.l2_gas_consumed.add(stats.l2_gas_consumed as u64, &[]);
    }
}
