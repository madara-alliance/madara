use crate::util::ExecutionStats;
use mc_analytics::{
    register_counter_metric_instrument, register_gauge_metric_instrument, register_histogram_metric_instrument,
};
use opentelemetry::metrics::{Counter, Gauge, Histogram};
use opentelemetry::{global, InstrumentationScope, KeyValue};
use tracing::warn;

pub struct BlockProductionMetrics {
    pub block_gauge: Gauge<u64>,
    pub block_counter: Counter<u64>,
    pub transaction_counter: Counter<u64>,
    pub block_production_time: Histogram<f64>,
    pub block_production_time_last: Gauge<f64>,
    pub block_close_time: Histogram<f64>,
    pub block_close_time_last: Gauge<f64>,
    // Batch execution stats
    pub batches_executed: Counter<u64>,
    pub txs_added_to_block: Counter<u64>,
    pub txs_executed: Counter<u64>,
    pub txs_reverted: Counter<u64>,
    pub txs_rejected: Counter<u64>,
    pub classes_declared: Counter<u64>,
    pub l2_gas_consumed: Counter<u64>,

    // Close block timing metrics
    pub close_block_total_duration: Histogram<f64>,
    pub close_block_total_last: Gauge<f64>,
    pub close_preconfirmed_duration: Histogram<f64>,
    pub close_preconfirmed_last: Gauge<f64>,
    pub executor_finalize_duration: Histogram<f64>,
    pub executor_finalize_last: Gauge<f64>,

    // Block data gauges
    pub block_event_count: Gauge<u64>,
    pub block_state_diff_length: Gauge<u64>,
    pub block_declared_classes_count: Gauge<u64>,
    pub block_deployed_contracts_count: Gauge<u64>,
    pub block_storage_diffs_count: Gauge<u64>,
    pub block_nonce_updates_count: Gauge<u64>,
    pub block_consumed_l1_nonces_count: Gauge<u64>,

    // Bouncer weights gauges
    pub block_bouncer_l1_gas: Gauge<u64>,
    pub block_bouncer_sierra_gas: Gauge<u64>,
    pub block_bouncer_n_events: Gauge<u64>,
    pub block_bouncer_message_segment_length: Gauge<u64>,
    pub block_bouncer_state_diff_size: Gauge<u64>,
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
        let block_production_time_last = register_gauge_metric_instrument(
            &meter,
            "block_production_time_last_seconds".to_string(),
            "Last block: time to produce a full block from start to close".to_string(),
            "s".to_string(),
        );
        let block_close_time = register_histogram_metric_instrument(
            &meter,
            "block_close_time".to_string(),
            "Time spent closing a block and persisting it".to_string(),
            "s".to_string(),
        );
        let block_close_time_last = register_gauge_metric_instrument(
            &meter,
            "block_close_time_last_seconds".to_string(),
            "Last block: time spent closing a block and persisting it".to_string(),
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

        // Close block timing metrics
        let close_block_total_duration = register_histogram_metric_instrument(
            &meter,
            "close_block_total_duration_seconds".to_string(),
            "Total time to close a block".to_string(),
            "s".to_string(),
        );
        let close_block_total_last = register_gauge_metric_instrument(
            &meter,
            "close_block_total_last_seconds".to_string(),
            "Last block: total time to close a block".to_string(),
            "s".to_string(),
        );
        let close_preconfirmed_duration = register_histogram_metric_instrument(
            &meter,
            "close_preconfirmed_duration_seconds".to_string(),
            "Time to close preconfirmed block with state diff".to_string(),
            "s".to_string(),
        );
        let close_preconfirmed_last = register_gauge_metric_instrument(
            &meter,
            "close_preconfirmed_last_seconds".to_string(),
            "Last block: time to close preconfirmed block with state diff".to_string(),
            "s".to_string(),
        );
        let executor_finalize_duration = register_histogram_metric_instrument(
            &meter,
            "executor_finalize_duration_seconds".to_string(),
            "Time for executor.finalize() to complete".to_string(),
            "s".to_string(),
        );
        let executor_finalize_last = register_gauge_metric_instrument(
            &meter,
            "executor_finalize_last_seconds".to_string(),
            "Last block: time for executor.finalize() to complete".to_string(),
            "s".to_string(),
        );

        // Block data gauges
        let block_event_count = register_gauge_metric_instrument(
            &meter,
            "block_event_count".to_string(),
            "Number of events in the closed block".to_string(),
            "events".to_string(),
        );
        let block_state_diff_length = register_gauge_metric_instrument(
            &meter,
            "block_state_diff_length".to_string(),
            "State diff length of the closed block".to_string(),
            "entries".to_string(),
        );
        let block_declared_classes_count = register_gauge_metric_instrument(
            &meter,
            "block_declared_classes_count".to_string(),
            "Number of declared classes in the closed block".to_string(),
            "classes".to_string(),
        );
        let block_deployed_contracts_count = register_gauge_metric_instrument(
            &meter,
            "block_deployed_contracts_count".to_string(),
            "Number of deployed contracts in the closed block".to_string(),
            "contracts".to_string(),
        );
        let block_storage_diffs_count = register_gauge_metric_instrument(
            &meter,
            "block_storage_diffs_count".to_string(),
            "Number of storage diff entries in the closed block".to_string(),
            "entries".to_string(),
        );
        let block_nonce_updates_count = register_gauge_metric_instrument(
            &meter,
            "block_nonce_updates_count".to_string(),
            "Number of nonce updates in the closed block".to_string(),
            "updates".to_string(),
        );
        let block_consumed_l1_nonces_count = register_gauge_metric_instrument(
            &meter,
            "block_consumed_l1_nonces_count".to_string(),
            "Number of L1 to L2 nonces consumed in the closed block".to_string(),
            "nonces".to_string(),
        );

        // Bouncer weights gauges
        let block_bouncer_l1_gas = register_gauge_metric_instrument(
            &meter,
            "block_bouncer_l1_gas".to_string(),
            "L1 gas consumed from bouncer weights".to_string(),
            "gas".to_string(),
        );
        let block_bouncer_sierra_gas = register_gauge_metric_instrument(
            &meter,
            "block_bouncer_sierra_gas".to_string(),
            "Sierra gas consumed from bouncer weights".to_string(),
            "gas".to_string(),
        );
        let block_bouncer_n_events = register_gauge_metric_instrument(
            &meter,
            "block_bouncer_n_events".to_string(),
            "Number of events from bouncer weights".to_string(),
            "events".to_string(),
        );
        let block_bouncer_message_segment_length = register_gauge_metric_instrument(
            &meter,
            "block_bouncer_message_segment_length".to_string(),
            "Message segment length from bouncer weights".to_string(),
            "length".to_string(),
        );
        let block_bouncer_state_diff_size = register_gauge_metric_instrument(
            &meter,
            "block_bouncer_state_diff_size".to_string(),
            "State diff size from bouncer weights".to_string(),
            "size".to_string(),
        );

        Self {
            block_gauge,
            block_counter,
            transaction_counter,
            block_production_time,
            block_production_time_last,
            block_close_time,
            block_close_time_last,
            batches_executed,
            txs_added_to_block,
            txs_executed,
            txs_reverted,
            txs_rejected,
            classes_declared,
            l2_gas_consumed,
            close_block_total_duration,
            close_block_total_last,
            close_preconfirmed_duration,
            close_preconfirmed_last,
            executor_finalize_duration,
            executor_finalize_last,
            block_event_count,
            block_state_diff_length,
            block_declared_classes_count,
            block_deployed_contracts_count,
            block_storage_diffs_count,
            block_nonce_updates_count,
            block_consumed_l1_nonces_count,
            block_bouncer_l1_gas,
            block_bouncer_sierra_gas,
            block_bouncer_n_events,
            block_bouncer_message_segment_length,
            block_bouncer_state_diff_size,
        }
    }

    pub fn record_execution_stats(&self, block_number: u64, stats: &ExecutionStats) {
        let attributes = [KeyValue::new("block_number", block_number.to_string())];
        self.batches_executed.add(stats.n_batches as u64, &attributes);
        self.txs_added_to_block.add(stats.n_added_to_block as u64, &attributes);
        self.txs_executed.add(stats.n_executed as u64, &attributes);
        self.txs_reverted.add(stats.n_reverted as u64, &attributes);
        self.txs_rejected.add(stats.n_rejected as u64, &attributes);
        self.classes_declared.add(stats.declared_classes as u64, &attributes);

        // Safely convert u128 to u64 for metrics, logging if truncation occurs
        let gas_consumed_u64 = stats.l2_gas_consumed.try_into().unwrap_or_else(|_| {
            warn!(
                "l2_gas_consumed ({}) exceeds u64::MAX ({}), saturating to u64::MAX for metrics",
                stats.l2_gas_consumed,
                u64::MAX
            );
            u64::MAX
        });
        self.l2_gas_consumed.add(gas_consumed_u64, &attributes);
    }
}
