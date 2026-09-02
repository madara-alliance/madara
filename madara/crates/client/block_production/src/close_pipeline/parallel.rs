//! Parallel root preparation and ordered close commit.

use super::super::*;
use mc_db::rocksdb::global_trie::in_memory::InMemoryRootComputation;

mod commit;
mod root;

const PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT: usize = 10;
static PARALLEL_ROOT_ACTIVE_JOBS: AtomicUsize = AtomicUsize::new(0);

/// Returns the number of blocking root computations currently running.
pub(super) fn active_parallel_root_jobs() -> usize {
    PARALLEL_ROOT_ACTIVE_JOBS.load(Ordering::Relaxed)
}

/// Balances the process-wide active-root diagnostic around one blocking job.
struct ParallelRootJobGuard;

impl ParallelRootJobGuard {
    /// Marks one root computation active and returns the new active count.
    fn acquire() -> (Self, usize) {
        let active_jobs = PARALLEL_ROOT_ACTIVE_JOBS.fetch_add(1, Ordering::Relaxed) + 1;
        (Self, active_jobs)
    }
}

impl Drop for ParallelRootJobGuard {
    /// Removes the completed blocking computation from the process-wide count.
    fn drop(&mut self) {
        PARALLEL_ROOT_ACTIVE_JOBS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Diagnostic timings and concurrency observations for one computed root.
#[derive(Clone, Copy, Debug, Default)]
struct ParallelMerkleSummary {
    base_snapshot_block: Option<u64>,
    squashed_block_count: usize,
    diff_start_block: Option<u64>,
    diff_end_block: u64,
    active_parallel_root_jobs_on_dispatch: usize,
    active_parallel_root_jobs_on_start: usize,
    active_parallel_root_jobs_before_finish: usize,
    active_parallel_root_jobs_after_finish: usize,
    root_spawn_blocking_queue: Duration,
    root_wait: Duration,
    squash_state_diffs: Duration,
    root_compute: Duration,
    root_total: Duration,
    boundary_flush: Option<Duration>,
    has_boundary_overlay: bool,
}

/// Keeps a computed root attached to the close payload that produced it.
pub(crate) struct ParallelComputedClosePayload {
    payload: QueuedClosePayload,
    root_response: InMemoryRootComputation,
    parallel_summary: ParallelMerkleSummary,
}

impl ParallelComputedClosePayload {
    /// Returns the attached block number for scheduler tests.
    #[cfg(test)]
    pub(crate) fn block_n(&self) -> u64 {
        self.payload.close_job_payload.block_n
    }
}

/// Values produced by the blocking root-computation phase.
struct RootComputationOutput {
    root_response: InMemoryRootComputation,
    active_on_start: usize,
    active_before_finish: usize,
    active_after_finish: usize,
    spawn_queue: Duration,
    squash: Duration,
    compute: Duration,
    total: Duration,
}

/// Values produced by the ordered DB commit phase.
struct ParallelDbCommitResult {
    block_result: mc_db::AddFullBlockResult,
    boundary_flush: Option<Duration>,
}

/// Creates a root result without DB work for finalizer scheduler tests.
#[cfg(test)]
pub(crate) fn parallel_computed_payload_for_test(payload: QueuedClosePayload) -> ParallelComputedClosePayload {
    let block_n = payload.close_job_payload.block_n;
    ParallelComputedClosePayload {
        payload,
        root_response: InMemoryRootComputation {
            block_n,
            contract_root: Felt::ZERO,
            class_root: Felt::ZERO,
            state_root: Felt::ZERO,
            timings: Default::default(),
            overlay: None,
        },
        parallel_summary: ParallelMerkleSummary::default(),
    }
}

/// Rejects a parallel queue that cannot satisfy the ten-block protocol lookback.
pub(crate) fn validate_parallel_queue_invariant(
    parallel_merkle_enabled: bool,
    close_queue_capacity: usize,
) -> anyhow::Result<()> {
    if parallel_merkle_enabled && close_queue_capacity < PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT {
        anyhow::bail!(
            "QueueInvariantViolated: configured capacity {} is below protocol minimum {}",
            close_queue_capacity,
            PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT
        );
    }
    Ok(())
}

/// Removes state diffs already incorporated into a durable boundary snapshot.
pub(crate) fn prune_diffs_since_snapshot(diffs_since_snapshot: &mut Vec<(u64, StateDiff)>, completed_block_n: u64) {
    diffs_since_snapshot.retain(|(block_n, _)| *block_n > completed_block_n);
}

/// Returns a contiguous diff span from the selected snapshot through the target.
pub(crate) fn collect_diffs_for_root_from_base(
    diffs_since_snapshot: &[(u64, StateDiff)],
    base_block_n: Option<u64>,
    target_block_n: u64,
) -> anyhow::Result<Vec<StateDiff>> {
    let mut expected = base_block_n.map_or(0, |block_n| block_n.saturating_add(1));
    let mut collected = Vec::new();

    for (block_n, state_diff) in diffs_since_snapshot.iter().filter(|(block_n, _)| *block_n <= target_block_n) {
        if *block_n < expected {
            continue;
        }
        anyhow::ensure!(
            *block_n == expected,
            "Missing tracked state diff for block #{expected} while preparing root for block #{target_block_n} from base {base_block_n:?}"
        );
        collected.push(state_diff.clone());
        expected = expected.saturating_add(1);
    }
    anyhow::ensure!(
        expected == target_block_n.saturating_add(1),
        "Incomplete tracked state diffs for root of block #{target_block_n} from base {base_block_n:?}"
    );
    Ok(collected)
}
