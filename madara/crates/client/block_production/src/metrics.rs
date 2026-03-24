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
    pub block_execution_duration: Histogram<f64>,
    pub block_execution_last: Gauge<f64>,
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
    pub close_end_to_end_duration: Histogram<f64>,
    pub close_end_to_end_last: Gauge<f64>,
    pub close_post_execution_duration: Histogram<f64>,
    pub close_post_execution_last: Gauge<f64>,
    pub close_commit_stage_duration: Histogram<f64>,
    pub close_commit_stage_last: Gauge<f64>,
    pub close_block_enqueue_duration: Histogram<f64>,
    pub close_block_enqueue_last: Gauge<f64>,
    pub executor_batch_execution_duration: Histogram<f64>,
    pub executor_batch_execution_last: Gauge<f64>,
    pub executor_inter_batch_wait_duration: Histogram<f64>,
    pub executor_inter_batch_wait_last: Gauge<f64>,
    pub executor_to_main_delivery_duration: Histogram<f64>,
    pub executor_to_main_delivery_last: Gauge<f64>,
    pub executor_to_close_queue_duration: Histogram<f64>,
    pub executor_to_close_queue_last: Gauge<f64>,
    pub executor_finalize_duration: Histogram<f64>,
    pub executor_finalize_last: Gauge<f64>,
    pub batcher_batch_wait_duration: Histogram<f64>,
    pub batcher_batch_wait_last: Gauge<f64>,
    pub batcher_output_backpressure_duration: Histogram<f64>,
    pub batcher_output_backpressure_last: Gauge<f64>,
    pub parallel_root_spawn_blocking_queue_duration: Histogram<f64>,
    pub parallel_root_spawn_blocking_queue_last: Gauge<f64>,
    pub parallel_root_compute_duration: Histogram<f64>,
    pub parallel_root_compute_last: Gauge<f64>,
    pub parallel_root_total_duration: Histogram<f64>,
    pub parallel_root_total_last: Gauge<f64>,
    pub parallel_root_await_duration: Histogram<f64>,
    pub parallel_root_await_last: Gauge<f64>,
    pub parallel_root_ready_to_commit_wait_duration: Histogram<f64>,
    pub parallel_root_ready_to_commit_wait_last: Gauge<f64>,
    pub parallel_root_failures_total: Counter<u64>,
    pub close_queue_enqueue_failures_total: Counter<u64>,
    pub close_job_failures_total: Counter<u64>,
    pub close_queue_depth: Gauge<u64>,
    pub close_queue_enqueued_total: Counter<u64>,
    pub close_queue_dequeued_total: Counter<u64>,
    pub close_queue_wait_duration: Histogram<f64>,
    pub close_queue_wait_last: Gauge<f64>,
    pub close_queue_in_flight: Gauge<u64>,
    pub parallel_root_active_jobs: Gauge<u64>,
    pub parallel_root_waiting_jobs: Gauge<u64>,
    pub parallel_root_ready_to_commit_jobs: Gauge<u64>,
    pub stage_executing_blocks: Gauge<u64>,
    pub stage_pending_close_completions: Gauge<u64>,
    pub stage_diffs_since_snapshot: Gauge<u64>,
    pub stage_tracked_blocks_total: Gauge<u64>,

    // Block data gauges
    pub block_tx_count: Gauge<u64>,
    pub block_batches_executed_gauge: Gauge<u64>,
    pub block_txs_added_to_block_gauge: Gauge<u64>,
    pub block_txs_executed_gauge: Gauge<u64>,
    pub block_txs_reverted_gauge: Gauge<u64>,
    pub block_txs_rejected_gauge: Gauge<u64>,
    pub block_l2_gas_consumed_gauge: Gauge<u64>,
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
        let block_execution_duration = register_histogram_metric_instrument(
            &meter,
            "block_execution_duration_seconds".to_string(),
            "Total executor transaction execution time accumulated for a block".to_string(),
            "s".to_string(),
        );
        let block_execution_last = register_gauge_metric_instrument(
            &meter,
            "block_execution_last_seconds".to_string(),
            "Last block: total executor transaction execution time".to_string(),
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
            "Deprecated: time spent in the close commit stage. Use close_end_to_end_duration_seconds for end-to-end close latency.".to_string(),
            "s".to_string(),
        );
        let close_block_total_last = register_gauge_metric_instrument(
            &meter,
            "close_block_total_last_seconds".to_string(),
            "Deprecated: last block close commit stage duration. Use close_end_to_end_last_seconds for end-to-end close latency.".to_string(),
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
        let close_end_to_end_duration = register_histogram_metric_instrument(
            &meter,
            "close_end_to_end_duration_seconds".to_string(),
            "Time from the main task receiving a close request until the block is fully closed".to_string(),
            "s".to_string(),
        );
        let close_end_to_end_last = register_gauge_metric_instrument(
            &meter,
            "close_end_to_end_last_seconds".to_string(),
            "Last block: end-to-end close latency from close request receipt to completion".to_string(),
            "s".to_string(),
        );
        let close_post_execution_duration = register_histogram_metric_instrument(
            &meter,
            "close_post_execution_duration_seconds".to_string(),
            "Time from the last executed batch finishing until the block is fully closed".to_string(),
            "s".to_string(),
        );
        let close_post_execution_last = register_gauge_metric_instrument(
            &meter,
            "close_post_execution_last_seconds".to_string(),
            "Last block: time from last executed batch to full close completion".to_string(),
            "s".to_string(),
        );
        let close_commit_stage_duration = register_histogram_metric_instrument(
            &meter,
            "close_commit_stage_duration_seconds".to_string(),
            "Time spent in the close commit stage after a worker begins processing the close job".to_string(),
            "s".to_string(),
        );
        let close_commit_stage_last = register_gauge_metric_instrument(
            &meter,
            "close_commit_stage_last_seconds".to_string(),
            "Last block: time spent in the close commit stage".to_string(),
            "s".to_string(),
        );
        let close_block_enqueue_duration = register_histogram_metric_instrument(
            &meter,
            "close_block_enqueue_duration_seconds".to_string(),
            "Time from the main task receiving a close request until the close job is enqueued".to_string(),
            "s".to_string(),
        );
        let close_block_enqueue_last = register_gauge_metric_instrument(
            &meter,
            "close_block_enqueue_last_seconds".to_string(),
            "Last block: time from close request receipt until queue enqueue".to_string(),
            "s".to_string(),
        );
        let executor_batch_execution_duration = register_histogram_metric_instrument(
            &meter,
            "executor_batch_execution_duration_seconds".to_string(),
            "Time spent by executor thread executing a single batch".to_string(),
            "s".to_string(),
        );
        let executor_batch_execution_last = register_gauge_metric_instrument(
            &meter,
            "executor_batch_execution_last_seconds".to_string(),
            "Last batch: time spent by executor thread executing batch".to_string(),
            "s".to_string(),
        );
        let executor_inter_batch_wait_duration = register_histogram_metric_instrument(
            &meter,
            "executor_inter_batch_wait_duration_seconds".to_string(),
            "Time between the previous batch finishing and the next batch starting execution".to_string(),
            "s".to_string(),
        );
        let executor_inter_batch_wait_last = register_gauge_metric_instrument(
            &meter,
            "executor_inter_batch_wait_last_seconds".to_string(),
            "Last batch: time between the previous batch finishing and the next batch starting".to_string(),
            "s".to_string(),
        );
        let executor_to_main_delivery_duration = register_histogram_metric_instrument(
            &meter,
            "executor_to_main_delivery_duration_seconds".to_string(),
            "Time between executor emitting BatchExecuted and main loop receiving it".to_string(),
            "s".to_string(),
        );
        let executor_to_main_delivery_last = register_gauge_metric_instrument(
            &meter,
            "executor_to_main_delivery_last_seconds".to_string(),
            "Last batch: time between executor emit and main loop receive".to_string(),
            "s".to_string(),
        );
        let executor_to_close_queue_duration = register_histogram_metric_instrument(
            &meter,
            "executor_to_close_queue_duration_seconds".to_string(),
            "Time between the last batch finishing execution and the main task receiving the close request".to_string(),
            "s".to_string(),
        );
        let executor_to_close_queue_last = register_gauge_metric_instrument(
            &meter,
            "executor_to_close_queue_last_seconds".to_string(),
            "Last block: time from last execution completion to close request receipt".to_string(),
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
        let batcher_batch_wait_duration = register_histogram_metric_instrument(
            &meter,
            "batcher_batch_wait_duration_seconds".to_string(),
            "Time the batcher waited for input before a batch became ready".to_string(),
            "s".to_string(),
        );
        let batcher_batch_wait_last = register_gauge_metric_instrument(
            &meter,
            "batcher_batch_wait_last_seconds".to_string(),
            "Last batch: time the batcher waited before a batch became ready".to_string(),
            "s".to_string(),
        );
        let batcher_output_backpressure_duration = register_histogram_metric_instrument(
            &meter,
            "batcher_output_backpressure_duration_seconds".to_string(),
            "Time the batcher waited for output channel capacity before it could enqueue a batch".to_string(),
            "s".to_string(),
        );
        let batcher_output_backpressure_last = register_gauge_metric_instrument(
            &meter,
            "batcher_output_backpressure_last_seconds".to_string(),
            "Last batch: time the batcher waited for output channel capacity".to_string(),
            "s".to_string(),
        );
        let parallel_root_spawn_blocking_queue_duration = register_histogram_metric_instrument(
            &meter,
            "parallel_root_spawn_blocking_queue_duration_seconds".to_string(),
            "Time root task waits before spawn_blocking closure starts".to_string(),
            "s".to_string(),
        );
        let parallel_root_spawn_blocking_queue_last = register_gauge_metric_instrument(
            &meter,
            "parallel_root_spawn_blocking_queue_last_seconds".to_string(),
            "Last block: root task wait before spawn_blocking starts".to_string(),
            "s".to_string(),
        );
        let parallel_root_compute_duration = register_histogram_metric_instrument(
            &meter,
            "parallel_root_compute_duration_seconds".to_string(),
            "Time spent computing parallel merkle root once closure starts".to_string(),
            "s".to_string(),
        );
        let parallel_root_compute_last = register_gauge_metric_instrument(
            &meter,
            "parallel_root_compute_last_seconds".to_string(),
            "Last block: time spent computing parallel merkle root".to_string(),
            "s".to_string(),
        );
        let parallel_root_total_duration = register_histogram_metric_instrument(
            &meter,
            "parallel_root_total_duration_seconds".to_string(),
            "Total time from dispatch_root to root result availability".to_string(),
            "s".to_string(),
        );
        let parallel_root_total_last = register_gauge_metric_instrument(
            &meter,
            "parallel_root_total_last_seconds".to_string(),
            "Last block: total time from dispatch_root to root result".to_string(),
            "s".to_string(),
        );
        let parallel_root_await_duration = register_histogram_metric_instrument(
            &meter,
            "parallel_root_await_duration_seconds".to_string(),
            "Wall time spent awaiting a pre-dispatched root task handle until the result is available".to_string(),
            "s".to_string(),
        );
        let parallel_root_await_last = register_gauge_metric_instrument(
            &meter,
            "parallel_root_await_last_seconds".to_string(),
            "Last block: wall time spent awaiting root task result availability".to_string(),
            "s".to_string(),
        );
        let parallel_root_ready_to_commit_wait_duration = register_histogram_metric_instrument(
            &meter,
            "parallel_root_ready_to_commit_wait_duration_seconds".to_string(),
            "Time a completed root result waits before the ordered commit stage starts".to_string(),
            "s".to_string(),
        );
        let parallel_root_ready_to_commit_wait_last = register_gauge_metric_instrument(
            &meter,
            "parallel_root_ready_to_commit_wait_last_seconds".to_string(),
            "Last block: time a completed root result waited for ordered commit".to_string(),
            "s".to_string(),
        );
        let parallel_root_failures_total = register_counter_metric_instrument(
            &meter,
            "parallel_root_failures_total".to_string(),
            "Count of parallel root computation failures".to_string(),
            "failure".to_string(),
        );
        let close_queue_enqueue_failures_total = register_counter_metric_instrument(
            &meter,
            "close_queue_enqueue_failures_total".to_string(),
            "Count of close queue enqueue failures".to_string(),
            "failure".to_string(),
        );
        let close_job_failures_total = register_counter_metric_instrument(
            &meter,
            "close_job_failures_total".to_string(),
            "Count of close jobs that failed in the finalizer pipeline".to_string(),
            "failure".to_string(),
        );
        let close_queue_depth = register_gauge_metric_instrument(
            &meter,
            "close_queue_depth".to_string(),
            "Current number of pending close jobs in the queue".to_string(),
            "job".to_string(),
        );
        let close_queue_enqueued_total = register_counter_metric_instrument(
            &meter,
            "close_queue_enqueued_total".to_string(),
            "Total number of close jobs enqueued".to_string(),
            "job".to_string(),
        );
        let close_queue_dequeued_total = register_counter_metric_instrument(
            &meter,
            "close_queue_dequeued_total".to_string(),
            "Total number of close jobs dequeued/completed".to_string(),
            "job".to_string(),
        );
        let close_queue_wait_duration = register_histogram_metric_instrument(
            &meter,
            "close_queue_wait_duration_seconds".to_string(),
            "Time close jobs wait in the finalizer queue before processing starts".to_string(),
            "s".to_string(),
        );
        let close_queue_wait_last = register_gauge_metric_instrument(
            &meter,
            "close_queue_wait_last_seconds".to_string(),
            "Last close job: time spent waiting in the finalizer queue before processing starts".to_string(),
            "s".to_string(),
        );
        let close_queue_in_flight = register_gauge_metric_instrument(
            &meter,
            "close_queue_in_flight".to_string(),
            "Number of close jobs currently inside the finalizer pipeline".to_string(),
            "job".to_string(),
        );
        let parallel_root_active_jobs = register_gauge_metric_instrument(
            &meter,
            "parallel_root_active_jobs".to_string(),
            "Number of parallel merkle root computations currently running".to_string(),
            "job".to_string(),
        );
        let parallel_root_waiting_jobs = register_gauge_metric_instrument(
            &meter,
            "parallel_root_waiting_jobs".to_string(),
            "Number of close jobs waiting for a root worker slot".to_string(),
            "job".to_string(),
        );
        let parallel_root_ready_to_commit_jobs = register_gauge_metric_instrument(
            &meter,
            "parallel_root_ready_to_commit_jobs".to_string(),
            "Number of completed root results waiting for ordered commit".to_string(),
            "job".to_string(),
        );
        let stage_executing_blocks = register_gauge_metric_instrument(
            &meter,
            "block_stage_executing_blocks".to_string(),
            "Number of blocks currently in execution stage".to_string(),
            "block".to_string(),
        );
        let stage_pending_close_completions = register_gauge_metric_instrument(
            &meter,
            "block_stage_pending_close_completions".to_string(),
            "Number of blocks waiting for close completion notification".to_string(),
            "block".to_string(),
        );
        let stage_diffs_since_snapshot = register_gauge_metric_instrument(
            &meter,
            "block_stage_diffs_since_snapshot".to_string(),
            "Number of block diffs retained since latest snapshot boundary".to_string(),
            "diff".to_string(),
        );
        let stage_tracked_blocks_total = register_gauge_metric_instrument(
            &meter,
            "block_stage_tracked_blocks_total".to_string(),
            "Total blocks tracked in in-memory block pipeline stages".to_string(),
            "block".to_string(),
        );

        // Block data gauges
        let block_tx_count = register_gauge_metric_instrument(
            &meter,
            "block_tx_count".to_string(),
            "Number of transactions in the last closed block".to_string(),
            "tx".to_string(),
        );
        let block_batches_executed_gauge = register_gauge_metric_instrument(
            &meter,
            "block_batches_executed_count".to_string(),
            "Number of executor batches in the last closed block".to_string(),
            "batch".to_string(),
        );
        let block_txs_added_to_block_gauge = register_gauge_metric_instrument(
            &meter,
            "block_txs_added_to_block_count".to_string(),
            "Number of transactions added to the last closed block".to_string(),
            "tx".to_string(),
        );
        let block_txs_executed_gauge = register_gauge_metric_instrument(
            &meter,
            "block_txs_executed_count".to_string(),
            "Number of transactions executed while producing the last closed block".to_string(),
            "tx".to_string(),
        );
        let block_txs_reverted_gauge = register_gauge_metric_instrument(
            &meter,
            "block_txs_reverted_count".to_string(),
            "Number of reverted transactions in the last closed block".to_string(),
            "tx".to_string(),
        );
        let block_txs_rejected_gauge = register_gauge_metric_instrument(
            &meter,
            "block_txs_rejected_count".to_string(),
            "Number of rejected transactions while producing the last closed block".to_string(),
            "tx".to_string(),
        );
        let block_l2_gas_consumed_gauge = register_gauge_metric_instrument(
            &meter,
            "block_l2_gas_consumed".to_string(),
            "L2 gas consumed by transactions in the last closed block".to_string(),
            "gas".to_string(),
        );
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
            block_execution_duration,
            block_execution_last,
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
            close_end_to_end_duration,
            close_end_to_end_last,
            close_post_execution_duration,
            close_post_execution_last,
            close_commit_stage_duration,
            close_commit_stage_last,
            close_block_enqueue_duration,
            close_block_enqueue_last,
            executor_batch_execution_duration,
            executor_batch_execution_last,
            executor_inter_batch_wait_duration,
            executor_inter_batch_wait_last,
            executor_to_main_delivery_duration,
            executor_to_main_delivery_last,
            executor_to_close_queue_duration,
            executor_to_close_queue_last,
            executor_finalize_duration,
            executor_finalize_last,
            batcher_batch_wait_duration,
            batcher_batch_wait_last,
            batcher_output_backpressure_duration,
            batcher_output_backpressure_last,
            parallel_root_spawn_blocking_queue_duration,
            parallel_root_spawn_blocking_queue_last,
            parallel_root_compute_duration,
            parallel_root_compute_last,
            parallel_root_total_duration,
            parallel_root_total_last,
            parallel_root_await_duration,
            parallel_root_await_last,
            parallel_root_ready_to_commit_wait_duration,
            parallel_root_ready_to_commit_wait_last,
            parallel_root_failures_total,
            close_queue_enqueue_failures_total,
            close_job_failures_total,
            close_queue_depth,
            close_queue_enqueued_total,
            close_queue_dequeued_total,
            close_queue_wait_duration,
            close_queue_wait_last,
            close_queue_in_flight,
            parallel_root_active_jobs,
            parallel_root_waiting_jobs,
            parallel_root_ready_to_commit_jobs,
            stage_executing_blocks,
            stage_pending_close_completions,
            stage_diffs_since_snapshot,
            stage_tracked_blocks_total,
            block_tx_count,
            block_batches_executed_gauge,
            block_txs_added_to_block_gauge,
            block_txs_executed_gauge,
            block_txs_reverted_gauge,
            block_txs_rejected_gauge,
            block_l2_gas_consumed_gauge,
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

    pub fn record_execution_stats(&self, stats: &ExecutionStats) {
        self.batches_executed.add(stats.n_batches as u64, &[]);
        self.txs_added_to_block.add(stats.n_added_to_block as u64, &[]);
        self.txs_executed.add(stats.n_executed as u64, &[]);
        self.txs_reverted.add(stats.n_reverted as u64, &[]);
        self.txs_rejected.add(stats.n_rejected as u64, &[]);
        self.classes_declared.add(stats.declared_classes as u64, &[]);

        // Safely convert u128 to u64 for metrics, logging if truncation occurs
        let gas_consumed_u64 = stats.l2_gas_consumed.try_into().unwrap_or_else(|_| {
            warn!(
                "l2_gas_consumed ({}) exceeds u64::MAX ({}), saturating to u64::MAX for metrics",
                stats.l2_gas_consumed,
                u64::MAX
            );
            u64::MAX
        });
        self.l2_gas_consumed.add(gas_consumed_u64, &[]);
    }
}
