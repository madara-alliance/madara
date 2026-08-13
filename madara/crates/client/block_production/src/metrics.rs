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
    pub executor_batch_execution_duration: Histogram<f64>,
    pub executor_batch_execution_last: Gauge<f64>,
    pub executor_to_main_delivery_duration: Histogram<f64>,
    pub executor_to_main_delivery_last: Gauge<f64>,
    pub executor_finalize_duration: Histogram<f64>,
    pub executor_finalize_last: Gauge<f64>,
    pub close_queue_enqueue_failures_total: Counter<u64>,
    pub close_job_failures_total: Counter<u64>,
    pub close_queue_depth: Gauge<u64>,
    pub close_queue_enqueued_total: Counter<u64>,
    pub close_queue_dequeued_total: Counter<u64>,
    pub close_queue_wait_duration: Histogram<f64>,
    pub close_queue_in_flight: Gauge<u64>,
    pub stage_executing_blocks: Gauge<u64>,
    pub stage_pending_close_completions: Gauge<u64>,
    pub stage_diffs_since_snapshot: Gauge<u64>,
    pub stage_tracked_blocks_total: Gauge<u64>,

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

    // ExecutionBox fallback / replay metrics (Step 6)
    /// Current execution mode gauge: 1 = mixed, 0 = blockifier_only.
    pub execution_mode_current: Gauge<u64>,
    /// Total fallback entries, labelled by reason.
    pub executionbox_fallback_total: Counter<u64>,
    /// Total manual disable calls.
    pub executionbox_manual_disable_total: Counter<u64>,
    /// Total manual enable calls.
    pub executionbox_manual_enable_total: Counter<u64>,
    /// Total tainted blocks discarded on fallback entry.
    pub executionbox_tainted_blocks_discarded_total: Counter<u64>,
    /// Total replay blocks enqueued on fallback entry.
    pub executionbox_replay_enqueued_blocks_total: Counter<u64>,
    /// Total replay blocks successfully processed.
    pub executionbox_replay_processed_blocks_total: Counter<u64>,
    /// Current replay queue length.
    pub executionbox_replay_queue_len: Gauge<u64>,
    /// Replay block execution duration.
    pub executionbox_replay_duration_seconds: Histogram<f64>,
    /// Enable attempt outcomes, labelled by result.
    pub executionbox_enable_result_total: Counter<u64>,
    /// V1 crash-gap visibility: constant 1 while replay queue is in-memory only.
    pub executionbox_replay_inmemory_mode: Gauge<u64>,
    /// V1 crash-gap visibility: incremented once on startup to signal recovery gap TODO.
    pub executionbox_replay_recovery_gap_todo_total: Counter<u64>,

    // Batcher routing metrics (Step 2, T-024)
    /// Total batcher cycles (one permit-acquire per cycle).
    pub batcher_cycles_total: Counter<u64>,
    /// Total transactions picked by the batcher from all streams.
    pub batcher_picked_txs_total: Counter<u64>,
    /// Total transactions routed to the Rust branch.
    pub batcher_routed_rust_total: Counter<u64>,
    /// Total transactions routed to the Blockifier branch.
    pub batcher_routed_blockifier_total: Counter<u64>,
    /// Total fallback routing decisions, labelled by reason (low-cardinality enum only).
    pub batcher_route_fallback_total: Counter<u64>,
    /// Multicall with mixed support (subset of batcher_route_fallback_total).
    pub batcher_multicall_partial_supported_total: Counter<u64>,
    /// Histogram of output batch size for the Rust branch (txs per cycle).
    pub batcher_output_rust_branch_size: Histogram<f64>,
    /// Histogram of output batch size for the Blockifier branch (txs per cycle).
    pub batcher_output_blockifier_branch_size: Histogram<f64>,

    // Comparator / re-execution metrics (Step 5, T-055)
    /// Total blocks submitted to the comparator (SD/ER comparison pair).
    pub comparator_blocks_compared_total: Counter<u64>,
    /// Total state-diff mismatches detected by the comparator.
    pub comparator_state_diff_mismatch_total: Counter<u64>,
    /// Total blocks where ER-x1 > ER-x2 (warn path, ExecutionBox still active).
    pub comparator_execbox_resources_gt_reexec_total: Counter<u64>,
    /// Total blocks where ER-x1 > block limit (fatal path).
    pub comparator_execbox_resources_gt_block_limit_total: Counter<u64>,
    /// Total ExecutionBox stop events, labelled by output/state/resource reason.
    pub comparator_executionbox_stop_total: Counter<u64>,
    /// Comparator pure-function compute duration per block.
    pub comparator_compare_duration_seconds: Histogram<f64>,
    /// Field mismatches labelled by typed category and policy.
    pub comparator_mismatches_total: Counter<u64>,
    /// Comparator decisions labelled by accept|accept_with_warn|stop_execution_box.
    pub comparator_decisions_total: Counter<u64>,
    /// Allowed actual-fee, fee-transfer amount, and approved fee-balance differences.
    pub comparator_allowed_fee_differences_total: Counter<u64>,
    /// Total re-execution cancellations, labelled by reason=epoch_cancelled|worker_error|shutdown.
    pub reexecution_cancelled_total: Counter<u64>,

    // Executor phase metrics (Step 3, T-033)
    /// Total routed payloads (RoutedBatchToExecute) received by the executor.
    pub executor_routed_payloads_total: Counter<u64>,
    /// Per-phase execution duration in seconds. Label: phase=blockifier|rust.
    pub executor_phase_duration_seconds: Histogram<f64>,
    /// Total transactions executed per phase. Label: phase=blockifier|rust.
    pub executor_phase_executed_txs_total: Counter<u64>,
    /// Total transactions deferred per phase and reason.
    /// Labels: phase=blockifier|rust, reason=block_full|force_close|deadline_close.
    pub executor_phase_deferred_txs_total: Counter<u64>,
    /// Total block-full events per phase. Label: phase=blockifier|rust.
    pub executor_block_full_total: Counter<u64>,
    /// Total RustExec capacity rejections. Label: reason=limit_exceeded|resource_error.
    pub executor_rust_capacity_reject_total: Counter<u64>,
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
            "Time close jobs wait in queue before processing".to_string(),
            "s".to_string(),
        );
        let close_queue_in_flight = register_gauge_metric_instrument(
            &meter,
            "close_queue_in_flight".to_string(),
            "Number of close jobs currently being processed".to_string(),
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

        // ExecutionBox fallback / replay metrics
        let execution_mode_current = register_gauge_metric_instrument(
            &meter,
            "execution_mode_current".to_string(),
            "Current execution mode: 1=mixed, 0=blockifier_only".to_string(),
            "mode".to_string(),
        );
        let executionbox_fallback_total = register_counter_metric_instrument(
            &meter,
            "executionbox_fallback_total".to_string(),
            "Total fallback entries labelled by reason".to_string(),
            "event".to_string(),
        );
        let executionbox_manual_disable_total = register_counter_metric_instrument(
            &meter,
            "executionbox_manual_disable_total".to_string(),
            "Total manual executionbox disable calls".to_string(),
            "event".to_string(),
        );
        let executionbox_manual_enable_total = register_counter_metric_instrument(
            &meter,
            "executionbox_manual_enable_total".to_string(),
            "Total manual executionbox enable calls".to_string(),
            "event".to_string(),
        );
        let executionbox_tainted_blocks_discarded_total = register_counter_metric_instrument(
            &meter,
            "executionbox_tainted_blocks_discarded_total".to_string(),
            "Total tainted blocks discarded on fallback entry".to_string(),
            "block".to_string(),
        );
        let executionbox_replay_enqueued_blocks_total = register_counter_metric_instrument(
            &meter,
            "executionbox_replay_enqueued_blocks_total".to_string(),
            "Total replay blocks enqueued on fallback entry".to_string(),
            "block".to_string(),
        );
        let executionbox_replay_processed_blocks_total = register_counter_metric_instrument(
            &meter,
            "executionbox_replay_processed_blocks_total".to_string(),
            "Total replay blocks successfully processed".to_string(),
            "block".to_string(),
        );
        let executionbox_replay_queue_len = register_gauge_metric_instrument(
            &meter,
            "executionbox_replay_queue_len".to_string(),
            "Current replay queue length".to_string(),
            "block".to_string(),
        );
        let executionbox_replay_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "executionbox_replay_duration_seconds".to_string(),
            "Replay block execution duration".to_string(),
            "s".to_string(),
        );
        let executionbox_enable_result_total = register_counter_metric_instrument(
            &meter,
            "executionbox_enable_result_total".to_string(),
            "Enable attempt outcomes labelled by result".to_string(),
            "event".to_string(),
        );
        let executionbox_replay_inmemory_mode = register_gauge_metric_instrument(
            &meter,
            "executionbox_replay_inmemory_mode".to_string(),
            "V1: constant 1 while replay queue is in-memory only".to_string(),
            "mode".to_string(),
        );
        let executionbox_replay_recovery_gap_todo_total = register_counter_metric_instrument(
            &meter,
            "executionbox_replay_recovery_gap_todo_total".to_string(),
            "V1: incremented on startup to signal in-memory replay crash-gap TODO".to_string(),
            "event".to_string(),
        );

        // Batcher routing metrics (T-024)
        let batcher_cycles_total = register_counter_metric_instrument(
            &meter,
            "batcher_cycles_total".to_string(),
            "Total batcher permit cycles".to_string(),
            "cycle".to_string(),
        );
        let batcher_picked_txs_total = register_counter_metric_instrument(
            &meter,
            "batcher_picked_txs_total".to_string(),
            "Total transactions picked by batcher from all streams".to_string(),
            "tx".to_string(),
        );
        let batcher_routed_rust_total = register_counter_metric_instrument(
            &meter,
            "batcher_routed_rust_total".to_string(),
            "Total transactions routed to the Rust branch".to_string(),
            "tx".to_string(),
        );
        let batcher_routed_blockifier_total = register_counter_metric_instrument(
            &meter,
            "batcher_routed_blockifier_total".to_string(),
            "Total transactions routed to the Blockifier branch".to_string(),
            "tx".to_string(),
        );
        let batcher_route_fallback_total = register_counter_metric_instrument(
            &meter,
            "batcher_route_fallback_total".to_string(),
            "Total Blockifier fallback routing decisions labelled by reason".to_string(),
            "tx".to_string(),
        );
        let batcher_multicall_partial_supported_total = register_counter_metric_instrument(
            &meter,
            "batcher_multicall_partial_supported_total".to_string(),
            "Total transactions with mixed multicall support (Blockifier fallback)".to_string(),
            "tx".to_string(),
        );
        let batcher_output_rust_branch_size = register_histogram_metric_instrument(
            &meter,
            "batcher_output_rust_branch_size_txs".to_string(),
            "Histogram of Rust branch size per batcher cycle".to_string(),
            "tx".to_string(),
        );
        let batcher_output_blockifier_branch_size = register_histogram_metric_instrument(
            &meter,
            "batcher_output_blockifier_branch_size_txs".to_string(),
            "Histogram of Blockifier branch size per batcher cycle".to_string(),
            "tx".to_string(),
        );

        // Comparator / re-execution metrics (T-055)
        let comparator_blocks_compared_total = register_counter_metric_instrument(
            &meter,
            "comparator_blocks_compared_total".to_string(),
            "Total blocks submitted to the comparator".to_string(),
            "block".to_string(),
        );
        let comparator_state_diff_mismatch_total = register_counter_metric_instrument(
            &meter,
            "comparator_state_diff_mismatch_total".to_string(),
            "Total state-diff mismatches detected by the comparator".to_string(),
            "event".to_string(),
        );
        let comparator_execbox_resources_gt_reexec_total = register_counter_metric_instrument(
            &meter,
            "comparator_execbox_resources_gt_reexec_total".to_string(),
            "Total blocks where ER-x1 > ER-x2 (warn path)".to_string(),
            "event".to_string(),
        );
        let comparator_execbox_resources_gt_block_limit_total = register_counter_metric_instrument(
            &meter,
            "comparator_execbox_resources_gt_block_limit_total".to_string(),
            "Total blocks where ER-x1 > block limit (fatal path)".to_string(),
            "event".to_string(),
        );
        let comparator_executionbox_stop_total = register_counter_metric_instrument(
            &meter,
            "comparator_executionbox_stop_total".to_string(),
            "Total ExecutionBox stop events labelled by reason".to_string(),
            "event".to_string(),
        );
        let comparator_compare_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "comparator_compare_duration_seconds".to_string(),
            "Comparator pure-function compute duration per block".to_string(),
            "s".to_string(),
        );
        let comparator_mismatches_total = register_counter_metric_instrument(
            &meter,
            "comparator_mismatches_total".to_string(),
            "Comparator field mismatches labelled by category and policy".to_string(),
            "mismatch".to_string(),
        );
        let comparator_decisions_total = register_counter_metric_instrument(
            &meter,
            "comparator_decisions_total".to_string(),
            "Comparator decisions labelled by outcome".to_string(),
            "decision".to_string(),
        );
        let comparator_allowed_fee_differences_total = register_counter_metric_instrument(
            &meter,
            "comparator_allowed_fee_differences_total".to_string(),
            "Allowed actual-fee, fee-transfer amount, and approved fee-balance differences".to_string(),
            "difference".to_string(),
        );
        let reexecution_cancelled_total = register_counter_metric_instrument(
            &meter,
            "reexecution_cancelled_total".to_string(),
            "Total re-execution cancellations labelled by reason".to_string(),
            "event".to_string(),
        );

        // Executor phase metrics (T-033)
        let executor_routed_payloads_total = register_counter_metric_instrument(
            &meter,
            "executor_routed_payloads_total".to_string(),
            "Total RoutedBatchToExecute payloads received by the executor".to_string(),
            "payload".to_string(),
        );
        let executor_phase_duration_seconds = register_histogram_metric_instrument(
            &meter,
            "executor_phase_duration_seconds".to_string(),
            "Execution phase duration in seconds, labelled by phase=blockifier|rust".to_string(),
            "s".to_string(),
        );
        let executor_phase_executed_txs_total = register_counter_metric_instrument(
            &meter,
            "executor_phase_executed_txs_total".to_string(),
            "Total transactions executed per phase, labelled by phase=blockifier|rust".to_string(),
            "tx".to_string(),
        );
        let executor_phase_deferred_txs_total = register_counter_metric_instrument(
            &meter,
            "executor_phase_deferred_txs_total".to_string(),
            "Total transactions deferred per phase and reason".to_string(),
            "tx".to_string(),
        );
        let executor_block_full_total = register_counter_metric_instrument(
            &meter,
            "executor_block_full_total".to_string(),
            "Total block-full events per phase, labelled by phase=blockifier|rust".to_string(),
            "event".to_string(),
        );
        let executor_rust_capacity_reject_total = register_counter_metric_instrument(
            &meter,
            "executor_rust_capacity_reject_total".to_string(),
            "Total RustExec capacity rejections labelled by reason=limit_exceeded|resource_error".to_string(),
            "event".to_string(),
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
            executor_batch_execution_duration,
            executor_batch_execution_last,
            executor_to_main_delivery_duration,
            executor_to_main_delivery_last,
            executor_finalize_duration,
            executor_finalize_last,
            close_queue_enqueue_failures_total,
            close_job_failures_total,
            close_queue_depth,
            close_queue_enqueued_total,
            close_queue_dequeued_total,
            close_queue_wait_duration,
            close_queue_in_flight,
            stage_executing_blocks,
            stage_pending_close_completions,
            stage_diffs_since_snapshot,
            stage_tracked_blocks_total,
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
            execution_mode_current,
            executionbox_fallback_total,
            executionbox_manual_disable_total,
            executionbox_manual_enable_total,
            executionbox_tainted_blocks_discarded_total,
            executionbox_replay_enqueued_blocks_total,
            executionbox_replay_processed_blocks_total,
            executionbox_replay_queue_len,
            executionbox_replay_duration_seconds,
            executionbox_enable_result_total,
            executionbox_replay_inmemory_mode,
            executionbox_replay_recovery_gap_todo_total,
            batcher_cycles_total,
            batcher_picked_txs_total,
            batcher_routed_rust_total,
            batcher_routed_blockifier_total,
            batcher_route_fallback_total,
            batcher_multicall_partial_supported_total,
            batcher_output_rust_branch_size,
            batcher_output_blockifier_branch_size,
            comparator_blocks_compared_total,
            comparator_state_diff_mismatch_total,
            comparator_execbox_resources_gt_reexec_total,
            comparator_execbox_resources_gt_block_limit_total,
            comparator_executionbox_stop_total,
            comparator_compare_duration_seconds,
            comparator_mismatches_total,
            comparator_decisions_total,
            comparator_allowed_fee_differences_total,
            reexecution_cancelled_total,
            executor_routed_payloads_total,
            executor_phase_duration_seconds,
            executor_phase_executed_txs_total,
            executor_phase_deferred_txs_total,
            executor_block_full_total,
            executor_rust_capacity_reject_total,
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
