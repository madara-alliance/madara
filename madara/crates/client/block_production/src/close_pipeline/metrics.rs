//! Shared close-completion measurements and metrics.

use super::super::*;

/// Block contents and resource counts emitted by both close implementations.
pub(super) struct CloseBlockFacts {
    pub tx_count: usize,
    pub event_count: u64,
    pub state_diff_len: usize,
    pub declared_classes: usize,
    pub deployed_contracts: usize,
    pub storage_diffs: usize,
    pub nonce_updates: usize,
    pub consumed_l1_nonces: usize,
    pub bouncer_l1_gas: u64,
    pub bouncer_sierra_gas: u64,
    pub bouncer_n_events: u64,
    pub bouncer_message_segment_length: u64,
    pub bouncer_state_diff_size: u64,
}

impl CloseBlockFacts {
    /// Reads transaction/event counts and copies resource totals before DB commit.
    pub fn collect(
        state: &CurrentBlockState,
        state_diff: &StateDiff,
        summary: &BlockExecutionSummary,
    ) -> anyhow::Result<Self> {
        let view = state
            .backend
            .block_view_on_preconfirmed(state.block_number)
            .with_context(|| format!("No pre-confirmed block #{}", state.block_number))?;
        let event_count =
            view.borrow_content().executed_transactions().map(|tx| tx.transaction.receipt.events().len() as u64).sum();

        Ok(Self {
            tx_count: view.num_executed_transactions(),
            event_count,
            state_diff_len: state_diff.len(),
            declared_classes: state_diff.declared_classes.len(),
            deployed_contracts: state_diff.deployed_contracts.len(),
            storage_diffs: state_diff.storage_diffs.len(),
            nonce_updates: state_diff.nonces.len(),
            consumed_l1_nonces: state.consumed_core_contract_nonces.len(),
            bouncer_l1_gas: summary.bouncer_weights.l1_gas as u64,
            bouncer_sierra_gas: summary.bouncer_weights.sierra_gas.0,
            bouncer_n_events: summary.bouncer_weights.n_events as u64,
            bouncer_message_segment_length: summary.bouncer_weights.message_segment_length as u64,
            bouncer_state_diff_size: summary.bouncer_weights.state_diff_size as u64,
        })
    }

    /// Records shape and bouncer gauges that do not depend on close duration.
    pub fn record_gauges(&self, metrics: &BlockProductionMetrics) {
        metrics.block_declared_classes_count.record(self.declared_classes as u64, &[]);
        metrics.block_deployed_contracts_count.record(self.deployed_contracts as u64, &[]);
        metrics.block_storage_diffs_count.record(self.storage_diffs as u64, &[]);
        metrics.block_nonce_updates_count.record(self.nonce_updates as u64, &[]);
        metrics.block_state_diff_length.record(self.state_diff_len as u64, &[]);
        metrics.block_event_count.record(self.event_count, &[]);
        metrics.block_bouncer_l1_gas.record(self.bouncer_l1_gas, &[]);
        metrics.block_bouncer_sierra_gas.record(self.bouncer_sierra_gas, &[]);
        metrics.block_bouncer_n_events.record(self.bouncer_n_events, &[]);
        metrics.block_bouncer_message_segment_length.record(self.bouncer_message_segment_length, &[]);
        metrics.block_bouncer_state_diff_size.record(self.bouncer_state_diff_size, &[]);
        metrics.block_consumed_l1_nonces_count.record(self.consumed_l1_nonces as u64, &[]);
    }
}

/// Durations shared by serial and parallel close completion logs.
pub(super) struct CloseDurations {
    pub executor_to_close_queue: Option<Duration>,
    pub close_block_to_queue_enqueue: Duration,
    pub close_preconfirmed: Duration,
    pub close_commit_stage: Duration,
    pub close_end_to_end: Duration,
    pub close_post_execution: Option<Duration>,
    pub block_production: Duration,
}

impl CloseDurations {
    /// Derives every close duration from the timestamps carried by the queue payload.
    pub fn capture(
        state: &CurrentBlockState,
        last_execution_finished_at: Option<Instant>,
        close_block_received_at: Instant,
        enqueued_at: Instant,
        commit_started_at: Instant,
        close_preconfirmed: Duration,
    ) -> Self {
        let close_end_to_end = close_block_received_at.elapsed();
        Self {
            executor_to_close_queue: last_execution_finished_at
                .map(|finished| close_block_received_at.duration_since(finished)),
            close_block_to_queue_enqueue: enqueued_at.duration_since(close_block_received_at),
            close_preconfirmed,
            close_commit_stage: commit_started_at.elapsed(),
            close_end_to_end,
            close_post_execution: last_execution_finished_at
                .map(|finished| close_block_received_at.duration_since(finished) + close_end_to_end),
            block_production: state.block_start_time.elapsed(),
        }
    }

    /// Records the DB-close duration shared by both implementations.
    pub fn record_close_preconfirmed(&self, metrics: &BlockProductionMetrics) {
        metrics.close_preconfirmed_duration.record(self.close_preconfirmed.as_secs_f64(), &[]);
        metrics.close_preconfirmed_last.record(self.close_preconfirmed.as_secs_f64(), &[]);
    }
}

/// Records the canonical completion counters and latency histograms for one block.
pub(super) fn record_closed_block_summary_metrics(
    metrics: &BlockProductionMetrics,
    state: &CurrentBlockState,
    facts: &CloseBlockFacts,
    durations: &CloseDurations,
) {
    let exec_stats = &state.accumulated_stats;
    let l2_gas_consumed = saturating_u128_to_u64("block_l2_gas_consumed", exec_stats.l2_gas_consumed);

    metrics.block_execution_duration.record(exec_stats.exec_duration.as_secs_f64(), &[]);
    metrics.block_execution_last.record(exec_stats.exec_duration.as_secs_f64(), &[]);
    metrics.close_end_to_end_duration.record(durations.close_end_to_end.as_secs_f64(), &[]);
    metrics.close_end_to_end_last.record(durations.close_end_to_end.as_secs_f64(), &[]);
    if let Some(duration) = durations.close_post_execution {
        metrics.close_post_execution_duration.record(duration.as_secs_f64(), &[]);
        metrics.close_post_execution_last.record(duration.as_secs_f64(), &[]);
    }
    metrics.close_commit_stage_duration.record(durations.close_commit_stage.as_secs_f64(), &[]);
    metrics.close_commit_stage_last.record(durations.close_commit_stage.as_secs_f64(), &[]);

    // Compatibility metrics represent commit-stage duration.
    metrics.close_block_total_duration.record(durations.close_commit_stage.as_secs_f64(), &[]);
    metrics.close_block_total_last.record(durations.close_commit_stage.as_secs_f64(), &[]);
    metrics.block_close_time.record(durations.close_commit_stage.as_secs_f64(), &[]);
    metrics.block_close_time_last.record(durations.close_commit_stage.as_secs_f64(), &[]);

    metrics.block_counter.add(1, &[]);
    metrics.block_gauge.record(state.block_number, &[]);
    metrics.transaction_counter.add(facts.tx_count as u64, &[]);
    metrics.block_production_time.record(durations.block_production.as_secs_f64(), &[]);
    metrics.block_production_time_last.record(durations.block_production.as_secs_f64(), &[]);
    metrics.block_tx_count.record(facts.tx_count as u64, &[]);
    metrics.block_batches_executed_gauge.record(exec_stats.n_batches as u64, &[]);
    metrics.block_txs_added_to_block_gauge.record(exec_stats.n_added_to_block as u64, &[]);
    metrics.block_txs_executed_gauge.record(exec_stats.n_executed as u64, &[]);
    metrics.block_txs_reverted_gauge.record(exec_stats.n_reverted as u64, &[]);
    metrics.block_txs_rejected_gauge.record(exec_stats.n_rejected as u64, &[]);
    metrics.block_l2_gas_consumed_gauge.record(l2_gas_consumed, &[]);
}

/// Saturates a gas total that cannot be represented by the metrics backend.
fn saturating_u128_to_u64(metric_name: &str, value: u128) -> u64 {
    value.try_into().unwrap_or_else(|_| {
        tracing::warn!("{metric_name} ({value}) exceeds u64::MAX ({}), saturating to u64::MAX for metrics", u64::MAX);
        u64::MAX
    })
}
