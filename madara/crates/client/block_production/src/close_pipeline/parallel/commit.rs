//! Ordered block-parts, boundary, and confirmed-head commit phases.

use super::*;
use crate::close_pipeline::metrics::{record_closed_block_summary_metrics, CloseBlockFacts, CloseDurations};
use mc_db::rocksdb::global_trie::in_memory::BonsaiOverlay;

impl BlockProductionTask {
    /// Commits one prepared root through the strictly ordered DB stage.
    /// Completion is returned only after block parts and the confirmed projection are durable.
    pub(crate) async fn execute_close_payload_parallel_precomputed_job(
        metrics: Arc<BlockProductionMetrics>,
        computed: ParallelComputedClosePayload,
    ) -> anyhow::Result<CloseJobCompletion> {
        Self::execute_close_payload_parallel_precomputed(
            metrics,
            computed.payload,
            computed.root_response,
            computed.parallel_summary,
        )
        .await
    }

    /// Writes block parts, optional boundary durability, then the confirmed head.
    /// This ordering keeps crash recovery anchored to the last published confirmation.
    async fn execute_close_payload_parallel_precomputed(
        metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
        root_response: InMemoryRootComputation,
        mut parallel_summary: ParallelMerkleSummary,
    ) -> anyhow::Result<CloseJobCompletion> {
        let QueuedClosePayload {
            state,
            block_exec_summary,
            state_diff,
            parallel_merkle_flush_interval,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            ..
        } = payload;
        tracing::debug!("close_block_worker_started block_number={} parallel_merkle=true", state.block_number);

        let facts = CloseBlockFacts::collect(&state, &state_diff, &block_exec_summary)?;
        facts.record_gauges(&metrics);
        let has_boundary_overlay = root_response.overlay.is_some();
        let commit_started_at = Instant::now();
        let close_started_at = Instant::now();
        let backend = Arc::clone(&state.backend);
        let block_n = state.block_number;
        let root_base_block_n = parallel_summary.base_snapshot_block;
        let bouncer_weights = block_exec_summary.bouncer_weights;
        let db_commit = global_spawn_rayon_task(move || {
            commit_parallel_db(
                backend,
                block_n,
                bouncer_weights,
                state_diff,
                root_response,
                parallel_merkle_flush_interval,
                root_base_block_n,
            )
        })
        .await?;

        let durations = CloseDurations::capture(
            &state,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            commit_started_at,
            close_started_at.elapsed(),
        );
        durations.record_close_preconfirmed(&metrics);
        parallel_summary.boundary_flush = db_commit.boundary_flush;
        parallel_summary.has_boundary_overlay = has_boundary_overlay;
        parallel_summary.boundary_checkpoint_persisted = db_commit.durable_checkpoint_committed;
        log_parallel_close_complete(&state, &facts, &durations, &db_commit.block_result, &parallel_summary);
        record_closed_block_summary_metrics(&metrics, &state, &facts, &durations);
        Ok(CloseJobCompletion { block_n, durable_checkpoint_committed: db_commit.durable_checkpoint_committed })
    }
}

/// Executes the three crash-sensitive DB phases in their required order.
/// A failure stops before any later phase can publish a partially prepared block.
fn commit_parallel_db(
    backend: Arc<MadaraBackend>,
    block_n: u64,
    bouncer_weights: blockifier::bouncer::BouncerWeights,
    state_diff: StateDiff,
    root_response: InMemoryRootComputation,
    flush_interval: u64,
    root_base_block_n: Option<u64>,
) -> anyhow::Result<ParallelDbCommitResult> {
    let pipeline_started_at = Instant::now();
    tracing::debug!(
        "parallel_close_db_pipeline_started block_number={} state_diff_len={} has_boundary_overlay={}",
        block_n,
        state_diff.len(),
        root_response.overlay.is_some()
    );

    write_bouncer_weights(&backend, block_n, &bouncer_weights)?;
    let InMemoryRootComputation { state_root, timings, overlay, .. } = root_response;
    let block_result = write_block_parts(&backend, block_n, state_diff, state_root, timings)?;
    let boundary_flush = flush_boundary(&backend, block_n, flush_interval, root_base_block_n, overlay.as_ref())?;
    confirm_block(&backend, block_n)?;
    tracing::debug!(
        "parallel_close_db_pipeline_finished block_number={} total_duration_ms={}",
        block_n,
        pipeline_started_at.elapsed().as_secs_f64() * 1000.0
    );
    Ok(ParallelDbCommitResult {
        block_result,
        boundary_flush: boundary_flush.duration,
        durable_checkpoint_committed: boundary_flush.checkpoint_persisted,
    })
}

/// Describes whether an optional boundary overlay advanced durable trie state.
struct BoundaryFlushResult {
    duration: Option<Duration>,
    checkpoint_persisted: bool,
}

/// Persists bouncer weights required by SNOS before block parts are written.
/// The write is timed separately so close latency remains attributable.
fn write_bouncer_weights(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    bouncer_weights: &blockifier::bouncer::BouncerWeights,
) -> anyhow::Result<()> {
    let started_at = Instant::now();
    backend
        .write_access()
        .write_bouncer_weights(block_n, bouncer_weights)
        .context("Saving Bouncer Weights for SNOS")?;
    tracing::debug!(
        "parallel_close_phase_bouncer_write_done block_number={} duration_ms={}",
        block_n,
        started_at.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// Writes immutable block parts with the precomputed root without advancing head.
/// The resulting staged block remains externally invisible until confirmation.
fn write_block_parts(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    state_diff: StateDiff,
    state_root: Felt,
    timings: mc_db::rocksdb::global_trie::MerklizationTimings,
) -> anyhow::Result<mc_db::AddFullBlockResult> {
    let started_at = Instant::now();
    let result = backend
        .write_access()
        .write_preconfirmed_with_precomputed_root(true, block_n, state_diff, state_root, timings)
        .context("Closing preconfirmed block with precomputed root")?;
    tracing::debug!(
        "parallel_close_phase_write_parts_done block_number={} duration_ms={} merklization_ms={} commitments_ms={} block_hash_ms={} db_write_ms={}",
        block_n,
        started_at.elapsed().as_secs_f64() * 1000.0,
        result.timings.merklization.as_secs_f64() * 1000.0,
        result.timings.block_commitments_compute.as_secs_f64() * 1000.0,
        result.timings.block_hash_compute.as_secs_f64() * 1000.0,
        result.timings.db_write_block_parts.as_secs_f64() * 1000.0
    );
    Ok(result)
}

/// Flushes a boundary overlay and checkpoint before the confirmed head moves.
/// Non-boundary blocks intentionally skip this durability phase.
fn flush_boundary(
    backend: &Arc<MadaraBackend>,
    block_n: u64,
    flush_interval: u64,
    overlay_base_block_n: Option<u64>,
    overlay: Option<&BonsaiOverlay>,
) -> anyhow::Result<BoundaryFlushResult> {
    let Some(overlay) = overlay else {
        return Ok(BoundaryFlushResult { duration: None, checkpoint_persisted: false });
    };
    let started_at = Instant::now();
    let outcome = backend
        .db
        .flush_overlay_and_checkpoint(block_n, flush_interval, overlay_base_block_n, overlay)
        .context("Flushing boundary overlay and writing parallel-merkle checkpoint")?;
    if let mc_db::rocksdb::global_trie::in_memory::BoundaryFlushOutcome::StaleBaseSkipped { latest_checkpoint } =
        outcome
    {
        tracing::debug!(
            block_number = block_n,
            ?overlay_base_block_n,
            latest_checkpoint,
            "parallel_merkle_stale_boundary_overlay_skipped"
        );
        return Ok(BoundaryFlushResult { duration: None, checkpoint_persisted: false });
    }

    let duration = started_at.elapsed();
    tracing::debug!(
        "parallel_close_phase_boundary_flush_done block_number={} duration_ms={} contract_changes={} contract_storage_changes={} class_changes={} latest_checkpoint={:?} checkpoint_floor_for_block={:?}",
        block_n,
        duration.as_secs_f64() * 1000.0,
        overlay.contract_changed.len(),
        overlay.contract_storage_changed.len(),
        overlay.class_changed.len(),
        backend.db.get_parallel_merkle_latest_checkpoint().ok().flatten(),
        backend.db.get_parallel_merkle_checkpoint_floor(block_n).ok().flatten()
    );
    Ok(BoundaryFlushResult { duration: Some(duration), checkpoint_persisted: true })
}

/// Advances the authoritative confirmed head only after parts and durability succeed.
/// It also emits the block-production notification owned by the backend.
fn confirm_block(backend: &Arc<MadaraBackend>, block_n: u64) -> anyhow::Result<()> {
    let started_at = Instant::now();
    backend
        .write_access()
        .new_confirmed_block(block_n)
        .context("Advancing confirmed head after parallel close write/flush")?;
    let head = backend.chain_head_state();
    tracing::debug!(
        "parallel_close_phase_confirm_done block_number={} duration_ms={} confirmed_tip={:?} external_preconfirmed_tip={:?} internal_preconfirmed_tip={:?}",
        block_n,
        started_at.elapsed().as_secs_f64() * 1000.0,
        head.confirmed_tip,
        head.external_preconfirmed_tip,
        head.internal_preconfirmed_tip
    );
    Ok(())
}

/// Emits the detailed parallel close event from already-collected facts.
/// Keeping formatting separate leaves the commit path focused on state transitions.
fn log_parallel_close_complete(
    state: &CurrentBlockState,
    facts: &CloseBlockFacts,
    durations: &CloseDurations,
    db_result: &mc_db::AddFullBlockResult,
    parallel: &ParallelMerkleSummary,
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
        base_snapshot_block = ?parallel.base_snapshot_block,
        squashed_block_count = parallel.squashed_block_count,
        diff_start_block = ?parallel.diff_start_block,
        diff_end_block = parallel.diff_end_block,
        active_parallel_root_jobs_on_dispatch = parallel.active_parallel_root_jobs_on_dispatch,
        active_parallel_root_jobs_on_start = parallel.active_parallel_root_jobs_on_start,
        active_parallel_root_jobs_before_finish = parallel.active_parallel_root_jobs_before_finish,
        active_parallel_root_jobs_after_finish = parallel.active_parallel_root_jobs_after_finish,
        root_spawn_blocking_queue_ms = parallel.root_spawn_blocking_queue.as_secs_f64() * 1000.0,
        root_wait_ms = parallel.root_wait.as_secs_f64() * 1000.0,
        squash_state_diffs_ms = parallel.squash_state_diffs.as_secs_f64() * 1000.0,
        root_compute_ms = parallel.root_compute.as_secs_f64() * 1000.0,
        root_total_ms = parallel.root_total.as_secs_f64() * 1000.0,
        boundary_flush_ms = ?parallel.boundary_flush.map(|value| value.as_secs_f64() * 1000.0),
        has_boundary_overlay = parallel.has_boundary_overlay,
        boundary_checkpoint_persisted = parallel.boundary_checkpoint_persisted,
        parallel_merkle = true,
        "close_block_complete"
    );
}
