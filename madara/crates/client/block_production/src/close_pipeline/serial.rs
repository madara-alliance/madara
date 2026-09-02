//! Sequential close execution.

use super::super::*;
use super::metrics::{record_closed_block_summary_metrics, CloseBlockFacts, CloseDurations};

impl BlockProductionTask {
    /// Closes one queued block through the existing sequential Merkle/DB path.
    async fn execute_close_payload(
        metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
    ) -> anyhow::Result<CloseJobCompletion> {
        let QueuedClosePayload {
            state,
            block_exec_summary,
            state_diff,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            ..
        } = payload;
        tracing::debug!("close_block_worker_started block_number={} parallel_merkle=false", state.block_number);

        let facts = CloseBlockFacts::collect(&state, &state_diff, &block_exec_summary)?;
        facts.record_gauges(&metrics);
        let commit_started_at = Instant::now();
        let close_started_at = Instant::now();
        let db_result = Self::close_preconfirmed_block_with_state_diff(
            Arc::clone(&state.backend),
            state.block_number,
            &block_exec_summary.bouncer_weights,
            state_diff,
        )
        .await
        .context("Closing block")?;
        let durations = CloseDurations::capture(
            &state,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            commit_started_at,
            close_started_at.elapsed(),
        );
        durations.record_close_preconfirmed(&metrics);
        log_serial_close_complete(&state, &facts, &durations, &db_result);
        record_closed_block_summary_metrics(&metrics, &state, &facts, &durations);
        Ok(CloseJobCompletion { block_n: state.block_number })
    }

    /// Processes a boundary-limited serial batch and preserves input result order.
    pub(crate) async fn execute_close_payload_batch(
        metrics: Arc<BlockProductionMetrics>,
        payloads: Vec<QueuedClosePayload>,
    ) -> Vec<anyhow::Result<CloseJobCompletion>> {
        let mut results = Vec::with_capacity(payloads.len());
        for payload in payloads {
            results.push(Self::execute_close_payload(Arc::clone(&metrics), payload).await);
        }
        results
    }
}

/// Emits the detailed sequential close event from already-collected facts.
fn log_serial_close_complete(
    state: &CurrentBlockState,
    facts: &CloseBlockFacts,
    durations: &CloseDurations,
    db_result: &mc_db::AddFullBlockResult,
) {
    let exec = &state.accumulated_stats;
    let timings = &db_result.timings;
    tracing::info!(
        target: "close_block",
        block_number = state.block_number,
        tx_count = facts.tx_count,
        event_count = facts.event_count,
        close_block_total_ms = durations.close_end_to_end.as_secs_f64() * 1000.0,
        close_end_to_end_ms = durations.close_end_to_end.as_secs_f64() * 1000.0,
        close_commit_stage_ms = durations.close_commit_stage.as_secs_f64() * 1000.0,
        close_post_execution_ms = ?durations.close_post_execution.map(|value| value.as_secs_f64() * 1000.0),
        block_close_ms = durations.close_commit_stage.as_secs_f64() * 1000.0,
        close_preconfirmed_ms = durations.close_preconfirmed.as_secs_f64() * 1000.0,
        block_production_ms = durations.block_production.as_secs_f64() * 1000.0,
        block_lifetime_ms = durations.block_production.as_secs_f64() * 1000.0,
        execution_total_ms = exec.exec_duration.as_secs_f64() * 1000.0,
        executor_to_close_queue_ms = ?durations.executor_to_close_queue.map(|value| value.as_secs_f64() * 1000.0),
        close_block_to_queue_enqueue_ms = durations.close_block_to_queue_enqueue.as_secs_f64() * 1000.0,
        batches_executed = exec.n_batches,
        txs_added_to_block = exec.n_added_to_block,
        txs_executed = exec.n_executed,
        txs_reverted = exec.n_reverted,
        txs_rejected = exec.n_rejected,
        classes_declared = exec.declared_classes,
        l2_gas_consumed = exec.l2_gas_consumed,
        state_diff_len = facts.state_diff_len,
        declared_classes = facts.declared_classes,
        deployed_contracts = facts.deployed_contracts,
        storage_diffs = facts.storage_diffs,
        nonce_updates = facts.nonce_updates,
        consumed_l1_nonces = facts.consumed_l1_nonces,
        bouncer_l1_gas = facts.bouncer_l1_gas,
        bouncer_sierra_gas = facts.bouncer_sierra_gas,
        bouncer_n_events = facts.bouncer_n_events,
        bouncer_message_segment_length = facts.bouncer_message_segment_length,
        bouncer_state_diff_size = facts.bouncer_state_diff_size,
        get_full_block_ms = timings.get_full_block_with_classes.as_secs_f64() * 1000.0,
        commitments_ms = timings.block_commitments_compute.as_secs_f64() * 1000.0,
        merklization_ms = timings.merklization.as_secs_f64() * 1000.0,
        contract_trie_ms = timings.contract_trie_root.as_secs_f64() * 1000.0,
        class_trie_ms = timings.class_trie_root.as_secs_f64() * 1000.0,
        contract_storage_trie_commit_ms = timings.contract_storage_trie_commit.as_secs_f64() * 1000.0,
        contract_trie_commit_ms = timings.contract_trie_commit.as_secs_f64() * 1000.0,
        class_trie_commit_ms = timings.class_trie_commit.as_secs_f64() * 1000.0,
        block_hash_ms = timings.block_hash_compute.as_secs_f64() * 1000.0,
        db_write_ms = timings.db_write_block_parts.as_secs_f64() * 1000.0,
        parallel_merkle = false,
        "close_block_complete"
    );
}
