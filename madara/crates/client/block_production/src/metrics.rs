use crate::util::ExecutionStats;
use mc_telemetry::{
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
    pub executor_wait_for_work_duration: Histogram<f64>,
    pub executor_close_reason_total: Counter<u64>,
    pub replay_boundary_spillover_transactions_total: Counter<u64>,
    pub executor_inter_batch_wait_duration: Histogram<f64>,
    pub executor_to_main_delivery_duration: Histogram<f64>,
    pub executor_to_main_delivery_last: Gauge<f64>,
    pub executor_to_close_queue_duration: Histogram<f64>,
    pub executor_to_close_queue_last: Gauge<f64>,
    pub executor_finalize_duration: Histogram<f64>,
    pub executor_finalize_last: Gauge<f64>,
    pub batcher_batch_wait_duration: Histogram<f64>,
    pub batcher_output_backpressure_duration: Histogram<f64>,
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

/// Creates a counter while keeping string conversion out of the registration table.
/// The wrapper makes each metric declaration a single reviewable line.
fn register_counter(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Counter<u64> {
    register_counter_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Creates an integer gauge while keeping string conversion out of the registration table.
/// Count and capacity gauges use this variant to retain their natural value type.
fn register_gauge(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Gauge<u64> {
    register_gauge_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Creates a floating-point gauge while keeping string conversion out of the registration table.
/// Duration gauges use this variant so sub-second measurements retain their precision.
fn register_f64_gauge(meter: &opentelemetry::metrics::Meter, name: &str, description: &str, unit: &str) -> Gauge<f64> {
    register_gauge_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

/// Creates a histogram while keeping string conversion out of the registration table.
/// The wrapper makes each metric declaration a single reviewable line.
fn register_histogram(
    meter: &opentelemetry::metrics::Meter,
    name: &str,
    description: &str,
    unit: &str,
) -> Histogram<f64> {
    register_histogram_metric_instrument(meter, name.to_owned(), description.to_owned(), unit.to_owned())
}

impl BlockProductionMetrics {
    /// Registers the complete block-production metric set under one instrumentation scope.
    /// Metric names, descriptions, and units are kept stable for dashboards and alerts.
    pub fn register() -> Self {
        let meter = global::meter_with_scope(
            InstrumentationScope::builder("crates.block_production.opentelemetry")
                .with_attributes([KeyValue::new("crate", "block_production")])
                .build(),
        );

        Self {
            block_gauge: register_gauge(&meter, "block_produced_no", "A gauge to show block state at given time", "block"),
            block_counter: register_counter(&meter, "block_produced_count", "A counter to show block state at given time", "block"),
            transaction_counter: register_counter(&meter, "transaction_counter", "A counter to show transaction state for the given block", "transaction"),
            block_production_time: register_histogram(&meter, "block_production_time", "Time to produce a full block from start to close", "s"),
            block_production_time_last: register_f64_gauge(&meter, "block_production_time_last_seconds", "Last block: time to produce a full block from start to close", "s"),
            block_close_time: register_histogram(&meter, "block_close_time", "Time spent closing a block and persisting it", "s"),
            block_close_time_last: register_f64_gauge(&meter, "block_close_time_last_seconds", "Last block: time spent closing a block and persisting it", "s"),
            block_execution_duration: register_histogram(&meter, "block_execution_duration_seconds", "Total executor transaction execution time accumulated for a block", "s"),
            block_execution_last: register_f64_gauge(&meter, "block_execution_last_seconds", "Last block: total executor transaction execution time", "s"),
            batches_executed: register_counter(&meter, "batches_executed", "Number of batches executed during block production", "batch"),
            txs_added_to_block: register_counter(&meter, "txs_added_to_block", "Number of transactions successfully added to blocks", "tx"),
            txs_executed: register_counter(&meter, "txs_executed", "Total number of transactions executed", "tx"),
            txs_reverted: register_counter(&meter, "txs_reverted", "Number of reverted transactions (included but failed)", "tx"),
            txs_rejected: register_counter(&meter, "txs_rejected", "Number of rejected transactions (not included in block)", "tx"),
            classes_declared: register_counter(&meter, "classes_declared", "Number of classes declared", "class"),
            l2_gas_consumed: register_counter(&meter, "l2_gas_consumed", "Total L2 gas consumed by transactions", "gas"),
            close_block_total_duration: register_histogram(&meter, "close_block_total_duration_seconds", "Deprecated: time spent in the close commit stage. Use close_end_to_end_duration_seconds for end-to-end close latency.", "s"),
            close_block_total_last: register_f64_gauge(&meter, "close_block_total_last_seconds", "Deprecated: last block close commit stage duration. Use close_end_to_end_last_seconds for end-to-end close latency.", "s"),
            close_preconfirmed_duration: register_histogram(&meter, "close_preconfirmed_duration_seconds", "Time to close preconfirmed block with state diff", "s"),
            close_preconfirmed_last: register_f64_gauge(&meter, "close_preconfirmed_last_seconds", "Last block: time to close preconfirmed block with state diff", "s"),
            close_end_to_end_duration: register_histogram(&meter, "close_end_to_end_duration_seconds", "Time from the main task receiving a close request until the block is fully closed", "s"),
            close_end_to_end_last: register_f64_gauge(&meter, "close_end_to_end_last_seconds", "Last block: end-to-end close latency from close request receipt to completion", "s"),
            close_post_execution_duration: register_histogram(&meter, "close_post_execution_duration_seconds", "Time from the last executed batch finishing until the block is fully closed", "s"),
            close_post_execution_last: register_f64_gauge(&meter, "close_post_execution_last_seconds", "Last block: time from last executed batch to full close completion", "s"),
            close_commit_stage_duration: register_histogram(&meter, "close_commit_stage_duration_seconds", "Time spent in the close commit stage after a worker begins processing the close job", "s"),
            close_commit_stage_last: register_f64_gauge(&meter, "close_commit_stage_last_seconds", "Last block: time spent in the close commit stage", "s"),
            close_block_enqueue_duration: register_histogram(&meter, "close_block_enqueue_duration_seconds", "Time from the main task receiving a close request until the close job is enqueued", "s"),
            close_block_enqueue_last: register_f64_gauge(&meter, "close_block_enqueue_last_seconds", "Last block: time from close request receipt until queue enqueue", "s"),
            executor_batch_execution_duration: register_histogram(&meter, "executor_batch_execution_duration_seconds", "Time spent by executor thread executing a single batch", "s"),
            executor_batch_execution_last: register_f64_gauge(&meter, "executor_batch_execution_last_seconds", "Last batch: time spent by executor thread executing batch", "s"),
            executor_wait_for_work_duration: register_histogram(&meter, "executor_wait_for_work_duration_seconds", "Time spent waiting for the next batch or command before the executor can make progress.", "s"),
            executor_close_reason_total: register_counter(&meter, "executor_close_reason_total", "Number of executor block-close decisions grouped by the reason that triggered closing.", "close"),
            replay_boundary_spillover_transactions_total: register_counter(&meter, "replay_boundary_spillover_transactions_total", "Number of transactions deferred into the next block because a replay boundary capped the current block.", "tx"),
            executor_inter_batch_wait_duration: register_histogram(&meter, "executor_inter_batch_wait_duration_seconds", "Time between the previous batch finishing and the next batch starting execution", "s"),
            executor_to_main_delivery_duration: register_histogram(&meter, "executor_to_main_delivery_duration_seconds", "Time between executor emitting BatchExecuted and main loop receiving it", "s"),
            executor_to_main_delivery_last: register_f64_gauge(&meter, "executor_to_main_delivery_last_seconds", "Last batch: time between executor emit and main loop receive", "s"),
            executor_to_close_queue_duration: register_histogram(&meter, "executor_to_close_queue_duration_seconds", "Time between the last batch finishing execution and the main task receiving the close request", "s"),
            executor_to_close_queue_last: register_f64_gauge(&meter, "executor_to_close_queue_last_seconds", "Last block: time from last execution completion to close request receipt", "s"),
            executor_finalize_duration: register_histogram(&meter, "executor_finalize_duration_seconds", "Time for executor.finalize() to complete", "s"),
            executor_finalize_last: register_f64_gauge(&meter, "executor_finalize_last_seconds", "Last block: time for executor.finalize() to complete", "s"),
            batcher_batch_wait_duration: register_histogram(&meter, "batcher_batch_wait_duration_seconds", "Time the batcher waited for input before a batch became ready", "s"),
            batcher_output_backpressure_duration: register_histogram(&meter, "batcher_output_backpressure_duration_seconds", "Time the batcher waited for output channel capacity before it could enqueue a batch", "s"),
            parallel_root_spawn_blocking_queue_duration: register_histogram(&meter, "parallel_root_spawn_blocking_queue_duration_seconds", "Time root task waits before spawn_blocking closure starts", "s"),
            parallel_root_spawn_blocking_queue_last: register_f64_gauge(&meter, "parallel_root_spawn_blocking_queue_last_seconds", "Last block: root task wait before spawn_blocking starts", "s"),
            parallel_root_compute_duration: register_histogram(&meter, "parallel_root_compute_duration_seconds", "Time spent computing parallel merkle root once closure starts", "s"),
            parallel_root_compute_last: register_f64_gauge(&meter, "parallel_root_compute_last_seconds", "Last block: time spent computing parallel merkle root", "s"),
            parallel_root_total_duration: register_histogram(&meter, "parallel_root_total_duration_seconds", "Total time from dispatch_root to root result availability", "s"),
            parallel_root_total_last: register_f64_gauge(&meter, "parallel_root_total_last_seconds", "Last block: total time from dispatch_root to root result", "s"),
            parallel_root_await_duration: register_histogram(&meter, "parallel_root_await_duration_seconds", "Wall time spent awaiting a pre-dispatched root task handle until the result is available", "s"),
            parallel_root_await_last: register_f64_gauge(&meter, "parallel_root_await_last_seconds", "Last block: wall time spent awaiting root task result availability", "s"),
            parallel_root_ready_to_commit_wait_duration: register_histogram(&meter, "parallel_root_ready_to_commit_wait_duration_seconds", "Time a completed root result waits before the ordered commit stage starts", "s"),
            parallel_root_ready_to_commit_wait_last: register_f64_gauge(&meter, "parallel_root_ready_to_commit_wait_last_seconds", "Last block: time a completed root result waited for ordered commit", "s"),
            parallel_root_failures_total: register_counter(&meter, "parallel_root_failures_total", "Count of parallel root computation failures", "failure"),
            close_queue_enqueue_failures_total: register_counter(&meter, "close_queue_enqueue_failures_total", "Count of close queue enqueue failures", "failure"),
            close_job_failures_total: register_counter(&meter, "close_job_failures_total", "Count of close jobs that failed in the finalizer pipeline", "failure"),
            close_queue_depth: register_gauge(&meter, "close_queue_depth", "Current number of pending close jobs in the queue", "job"),
            close_queue_enqueued_total: register_counter(&meter, "close_queue_enqueued_total", "Total number of close jobs enqueued", "job"),
            close_queue_dequeued_total: register_counter(&meter, "close_queue_dequeued_total", "Total number of close jobs dequeued/completed", "job"),
            close_queue_wait_duration: register_histogram(&meter, "close_queue_wait_duration_seconds", "Time close jobs wait in the finalizer queue before processing starts", "s"),
            close_queue_wait_last: register_f64_gauge(&meter, "close_queue_wait_last_seconds", "Last close job: time spent waiting in the finalizer queue before processing starts", "s"),
            close_queue_in_flight: register_gauge(&meter, "close_queue_in_flight", "Number of close jobs currently inside the finalizer pipeline", "job"),
            parallel_root_active_jobs: register_gauge(&meter, "parallel_root_active_jobs", "Number of parallel merkle root computations currently running", "job"),
            parallel_root_waiting_jobs: register_gauge(&meter, "parallel_root_waiting_jobs", "Number of close jobs waiting for a root worker slot", "job"),
            parallel_root_ready_to_commit_jobs: register_gauge(&meter, "parallel_root_ready_to_commit_jobs", "Number of completed root results waiting for ordered commit", "job"),
            stage_executing_blocks: register_gauge(&meter, "block_stage_executing_blocks", "Number of blocks currently in execution stage", "block"),
            stage_pending_close_completions: register_gauge(&meter, "block_stage_pending_close_completions", "Number of blocks waiting for close completion notification", "block"),
            stage_diffs_since_snapshot: register_gauge(&meter, "block_stage_diffs_since_snapshot", "Number of block diffs retained since latest snapshot boundary", "diff"),
            stage_tracked_blocks_total: register_gauge(&meter, "block_stage_tracked_blocks_total", "Total blocks tracked in in-memory block pipeline stages", "block"),
            block_tx_count: register_gauge(&meter, "block_tx_count", "Number of transactions in the last closed block", "tx"),
            block_batches_executed_gauge: register_gauge(&meter, "block_batches_executed_count", "Number of executor batches in the last closed block", "batch"),
            block_txs_added_to_block_gauge: register_gauge(&meter, "block_txs_added_to_block_count", "Number of transactions added to the last closed block", "tx"),
            block_txs_executed_gauge: register_gauge(&meter, "block_txs_executed_count", "Number of transactions executed while producing the last closed block", "tx"),
            block_txs_reverted_gauge: register_gauge(&meter, "block_txs_reverted_count", "Number of reverted transactions in the last closed block", "tx"),
            block_txs_rejected_gauge: register_gauge(&meter, "block_txs_rejected_count", "Number of rejected transactions while producing the last closed block", "tx"),
            block_l2_gas_consumed_gauge: register_gauge(&meter, "block_l2_gas_consumed", "L2 gas consumed by transactions in the last closed block", "gas"),
            block_event_count: register_gauge(&meter, "block_event_count", "Number of events in the closed block", "events"),
            block_state_diff_length: register_gauge(&meter, "block_state_diff_length", "State diff length of the closed block", "entries"),
            block_declared_classes_count: register_gauge(&meter, "block_declared_classes_count", "Number of declared classes in the closed block", "classes"),
            block_deployed_contracts_count: register_gauge(&meter, "block_deployed_contracts_count", "Number of deployed contracts in the closed block", "contracts"),
            block_storage_diffs_count: register_gauge(&meter, "block_storage_diffs_count", "Number of storage diff entries in the closed block", "entries"),
            block_nonce_updates_count: register_gauge(&meter, "block_nonce_updates_count", "Number of nonce updates in the closed block", "updates"),
            block_consumed_l1_nonces_count: register_gauge(&meter, "block_consumed_l1_nonces_count", "Number of L1 to L2 nonces consumed in the closed block", "nonces"),
            block_bouncer_l1_gas: register_gauge(&meter, "block_bouncer_l1_gas", "L1 gas consumed from bouncer weights", "gas"),
            block_bouncer_sierra_gas: register_gauge(&meter, "block_bouncer_sierra_gas", "Sierra gas consumed from bouncer weights", "gas"),
            block_bouncer_n_events: register_gauge(&meter, "block_bouncer_n_events", "Number of events from bouncer weights", "events"),
            block_bouncer_message_segment_length: register_gauge(&meter, "block_bouncer_message_segment_length", "Message segment length from bouncer weights", "length"),
            block_bouncer_state_diff_size: register_gauge(&meter, "block_bouncer_state_diff_size", "State diff size from bouncer weights", "size"),
        }
    }

    /// Records aggregate executor outcomes for one processed batch.
    /// Gas values that exceed the metric type saturate and emit a warning.
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
