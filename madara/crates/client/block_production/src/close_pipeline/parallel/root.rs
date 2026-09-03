//! Blocking root computation against a selected durable snapshot.

use super::*;
use mc_db::rocksdb::SnapshotRef;
use mp_chain_config::StarknetVersion;

/// Everything a blocking Merkle worker needs, captured before leaving Tokio.
struct RootComputationRequest {
    backend: Arc<MadaraBackend>,
    metrics: Arc<BlockProductionMetrics>,
    snapshot: SnapshotRef,
    state_diffs: Vec<StateDiff>,
    block_n: u64,
    base_block_n: Option<u64>,
    protocol_version: StarknetVersion,
    is_boundary: bool,
    compare_sequential: bool,
    dispatched_at: Instant,
}

/// Stable fields emitted when one blocking root computation completes.
struct RootFinishedLog {
    block_n: u64,
    base_block_n: Option<u64>,
    diff_count: usize,
    is_boundary: bool,
    squash: Duration,
    compute: Duration,
    total: Duration,
    active_before_finish: usize,
}

impl BlockProductionTask {
    /// Precomputes one block's Merkle root without mutating confirmed DB state.
    pub(crate) async fn compute_close_payload_parallel_root(
        metrics: Arc<BlockProductionMetrics>,
        mut payload: QueuedClosePayload,
    ) -> anyhow::Result<ParallelComputedClosePayload> {
        let block_n = payload.state.block_number;
        let base_block_n = payload.root_base_block_n;
        let state_diffs = mem::take(&mut payload.root_state_diffs);
        let snapshot = payload.root_snapshot.take().context("Missing root snapshot for parallel close job")?;
        let squashed_block_count = state_diffs.len();
        let diff_start_block = base_block_n.map(|base| base.saturating_add(1)).or(Some(block_n));
        let active_on_dispatch = active_parallel_root_jobs();
        log_root_dispatched(&payload, squashed_block_count, diff_start_block, active_on_dispatch);

        let wait_started_at = Instant::now();
        let dispatched_at = Instant::now();
        let request = RootComputationRequest {
            backend: Arc::clone(&payload.state.backend),
            metrics: Arc::clone(&metrics),
            snapshot,
            state_diffs,
            block_n,
            base_block_n,
            protocol_version: payload.protocol_version,
            is_boundary: payload.is_boundary,
            compare_sequential: payload.compare_parallel_with_sequential,
            dispatched_at,
        };
        let compute = spawn_root_computation(request).await?;
        let root_wait = wait_started_at.elapsed();
        metrics.parallel_root_await_duration.record(root_wait.as_secs_f64(), &[]);
        metrics.parallel_root_await_last.record(root_wait.as_secs_f64(), &[]);
        tracing::debug!(
            "parallel_root_await_finished block_number={} root_wait_ms={} real_parallel_merkle=true",
            block_n,
            root_wait.as_secs_f64() * 1000.0
        );

        let has_boundary_overlay = compute.root_response.overlay.is_some();
        Ok(ParallelComputedClosePayload {
            payload,
            root_response: compute.root_response,
            parallel_summary: ParallelMerkleSummary {
                base_snapshot_block: base_block_n,
                squashed_block_count,
                diff_start_block,
                diff_end_block: block_n,
                active_parallel_root_jobs_on_dispatch: active_on_dispatch,
                active_parallel_root_jobs_on_start: compute.active_on_start,
                active_parallel_root_jobs_before_finish: compute.active_before_finish,
                active_parallel_root_jobs_after_finish: compute.active_after_finish,
                root_spawn_blocking_queue: compute.spawn_queue,
                root_wait,
                squash_state_diffs: compute.squash,
                root_compute: compute.compute,
                root_total: compute.total,
                boundary_flush: None,
                has_boundary_overlay,
                boundary_checkpoint_persisted: false,
            },
        })
    }
}

/// Executes the blocking root job and maps panics/failures into one metric.
async fn spawn_root_computation(request: RootComputationRequest) -> anyhow::Result<RootComputationOutput> {
    let block_n = request.block_n;
    let metrics = Arc::clone(&request.metrics);
    let result = tokio::task::spawn_blocking(move || compute_root_blocking(request)).await.map_err(|error| {
        metrics.parallel_root_failures_total.add(1, &[]);
        anyhow::anyhow!("Parallel merkle blocking task panicked for block #{block_n}: {error:#}")
    })?;
    result
        .inspect_err(|_| metrics.parallel_root_failures_total.add(1, &[]))
        .with_context(|| format!("Parallel merkle root computation for block #{block_n}"))
}

/// Squashes cumulative diffs and computes a root against the selected snapshot.
fn compute_root_blocking(request: RootComputationRequest) -> anyhow::Result<RootComputationOutput> {
    let RootComputationRequest {
        backend,
        metrics,
        snapshot,
        state_diffs,
        block_n,
        base_block_n,
        protocol_version,
        is_boundary,
        compare_sequential,
        dispatched_at,
    } = request;
    let started_at = Instant::now();
    let spawn_queue = started_at.duration_since(dispatched_at);
    metrics.parallel_root_spawn_blocking_queue_duration.record(spawn_queue.as_secs_f64(), &[]);
    metrics.parallel_root_spawn_blocking_queue_last.record(spawn_queue.as_secs_f64(), &[]);
    let (guard, active_on_start) = ParallelRootJobGuard::acquire();
    tracing::debug!(
        "parallel_root_single_block_compute_started block_number={} base_snapshot_block={base_block_n:?} diff_count={} squashed_block_count={} include_overlay={} spawn_blocking_queue_ms={} active_parallel_root_jobs={}",
        block_n,
        state_diffs.len(),
        state_diffs.len(),
        is_boundary,
        spawn_queue.as_secs_f64() * 1000.0,
        active_on_start
    );

    let squash_started_at = Instant::now();
    let cumulative_diff = mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(state_diffs.iter());
    let squash = squash_started_at.elapsed();
    let compute_started_at = Instant::now();
    let root_response = backend.db.compute_root_from_selected_snapshot(
        base_block_n,
        snapshot,
        block_n,
        &cumulative_diff,
        protocol_version,
        is_boundary,
        compare_sequential,
    )?;
    let compute = compute_started_at.elapsed();
    let total = dispatched_at.elapsed();
    metrics.parallel_root_compute_duration.record(compute.as_secs_f64(), &[]);
    metrics.parallel_root_compute_last.record(compute.as_secs_f64(), &[]);
    metrics.parallel_root_total_duration.record(total.as_secs_f64(), &[]);
    metrics.parallel_root_total_last.record(total.as_secs_f64(), &[]);
    let active_before_finish = active_parallel_root_jobs();
    drop(guard);
    let active_after_finish = active_parallel_root_jobs();
    log_root_finished(RootFinishedLog {
        block_n,
        base_block_n,
        diff_count: state_diffs.len(),
        is_boundary,
        squash,
        compute,
        total,
        active_before_finish,
    });

    Ok(RootComputationOutput {
        root_response,
        active_on_start,
        active_before_finish,
        active_after_finish,
        spawn_queue,
        squash,
        compute,
        total,
    })
}

/// Logs dispatch metadata before the root job leaves the async scheduler.
fn log_root_dispatched(
    payload: &QueuedClosePayload,
    squashed_block_count: usize,
    diff_start_block: Option<u64>,
    active_jobs: usize,
) {
    tracing::debug!(
        "parallel_root_single_block_dispatch block_number={} base_snapshot_block={:?} diff_count={} squashed_block_count={} diff_start_block={diff_start_block:?} diff_end_block={} include_overlay={} active_parallel_root_jobs={}",
        payload.state.block_number,
        payload.root_base_block_n,
        squashed_block_count,
        squashed_block_count,
        payload.state.block_number,
        payload.is_boundary,
        active_jobs
    );
}

/// Logs timing and concurrency after one blocking root computation finishes.
fn log_root_finished(details: RootFinishedLog) {
    let RootFinishedLog {
        block_n,
        base_block_n,
        diff_count,
        is_boundary,
        squash,
        compute,
        total,
        active_before_finish,
    } = details;
    tracing::debug!(
        "parallel_root_single_block_compute_finished block_number={} base_snapshot_block={base_block_n:?} diff_count={} squashed_block_count={} include_overlay={} success=true squash_state_diffs_ms={} compute_ms={} total_ms={} active_parallel_root_jobs={}",
        block_n,
        diff_count,
        diff_count,
        is_boundary,
        squash.as_secs_f64() * 1000.0,
        compute.as_secs_f64() * 1000.0,
        total.as_secs_f64() * 1000.0,
        active_before_finish
    );
}
