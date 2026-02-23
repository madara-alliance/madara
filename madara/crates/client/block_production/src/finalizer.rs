use crate::{ParallelMerkleConfig, ParallelMerkleTrieLogMode};
use anyhow::Context;
use blockifier::bouncer::BouncerWeights;
use mc_db::{BonsaiOverlay, MadaraBackend, ParallelMerkleInMemoryTrieLogMode};
use mp_block::FullBlockWithoutCommitments;
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use tokio::{sync::mpsc, task::JoinSet};

/// Finalized block payload handed off from block production to the parallel finalizer.
///
/// This contains all data required by both finalizer lanes:
/// - DB staging lane (block/classes/weights/consumed nonces)
/// - Root compute lane (state diff for cumulative merklization)
#[derive(Debug)]
pub struct FinalizedBlockPayload {
    pub block_n: u64,
    pub block: FullBlockWithoutCommitments,
    pub classes: Vec<ConvertedClass>,
    pub state_diff: StateDiff,
    pub consumed_core_contract_nonces: HashSet<u64>,
    pub bouncer_weights: BouncerWeights,
}

#[derive(Clone)]
pub struct ParallelMerkleFinalizerHandle {
    sender: mpsc::Sender<FinalizedBlockPayload>,
}

impl ParallelMerkleFinalizerHandle {
    /// Enqueue a block for parallel finalization.
    ///
    /// Queue backpressure is controlled by `parallel_merkle.max_inflight`.
    pub async fn submit(&self, payload: FinalizedBlockPayload) -> anyhow::Result<()> {
        self.sender.send(payload).await.context("sending finalized block payload to parallel-merkle finalizer")
    }
}

#[derive(Debug, Clone)]
struct RootReady {
    root: Felt,
    overlay: Option<BonsaiOverlay>,
    is_boundary: bool,
}

#[derive(Debug, Clone)]
struct LaneFailure {
    lane: &'static str,
    message: String,
}

pub fn spawn_parallel_merkle_finalizer(
    backend: Arc<MadaraBackend>,
    config: ParallelMerkleConfig,
) -> anyhow::Result<ParallelMerkleFinalizerHandle> {
    let max_inflight =
        usize::try_from(config.max_inflight).context("parallel_merkle.max_inflight does not fit usize")?;
    let max_inflight = max_inflight.max(1);
    let (sender, receiver) = mpsc::channel(max_inflight);

    let latest_confirmed = backend.latest_confirmed_block_n();
    let next_confirm = latest_confirmed.map(|n| n + 1).unwrap_or(0);
    let latest_checkpoint =
        backend.get_parallel_merkle_latest_checkpoint().context("reading parallel merkle latest checkpoint")?;
    if let (Some(checkpoint), Some(confirmed)) = (latest_checkpoint, latest_confirmed) {
        if checkpoint < confirmed {
            tracing::warn!(
                latest_checkpoint = checkpoint,
                latest_confirmed = confirmed,
                "parallel finalizer bootstrap prefers latest confirmed as snapshot base to avoid stale checkpoint base"
            );
        }
    }
    let snapshot_base_block_n = initial_snapshot_base(latest_confirmed, latest_checkpoint);

    let trie_log_mode = map_trie_log_mode(config.trie_log_mode);
    let mut worker = ParallelMerkleFinalizerWorker {
        backend,
        flush_interval: config.flush_interval.max(2),
        trie_log_mode,
        next_confirm,
        snapshot_base_block_n,
        pending_epoch_diffs: BTreeMap::new(),
        persisted_ready: BTreeSet::new(),
        roots_ready: BTreeMap::new(),
        db_staging_jobs: JoinSet::new(),
        root_compute_jobs: JoinSet::new(),
        max_inflight_blocks: max_inflight,
        scheduled_blocks: BTreeSet::new(),
        failed_blocks: BTreeMap::new(),
    };
    tokio::spawn(async move {
        if let Err(error) = worker.run(receiver).await {
            tracing::error!(%error, "parallel merkle finalizer worker terminated with error");
        }
    });

    Ok(ParallelMerkleFinalizerHandle { sender })
}

fn initial_snapshot_base(latest_confirmed: Option<u64>, latest_checkpoint: Option<u64>) -> u64 {
    latest_confirmed.or(latest_checkpoint).unwrap_or(0)
}

fn map_trie_log_mode(mode: ParallelMerkleTrieLogMode) -> ParallelMerkleInMemoryTrieLogMode {
    match mode {
        ParallelMerkleTrieLogMode::Off => ParallelMerkleInMemoryTrieLogMode::Off,
        ParallelMerkleTrieLogMode::Checkpoint => ParallelMerkleInMemoryTrieLogMode::Checkpoint,
    }
}

struct ParallelMerkleFinalizerWorker {
    backend: Arc<MadaraBackend>,
    flush_interval: u64,
    trie_log_mode: ParallelMerkleInMemoryTrieLogMode,
    next_confirm: u64,
    snapshot_base_block_n: u64,
    pending_epoch_diffs: BTreeMap<u64, StateDiff>,
    persisted_ready: BTreeSet<u64>,
    roots_ready: BTreeMap<u64, RootReady>,
    db_staging_jobs: JoinSet<(u64, anyhow::Result<()>)>,
    root_compute_jobs: JoinSet<(u64, anyhow::Result<RootReady>)>,
    max_inflight_blocks: usize,
    scheduled_blocks: BTreeSet<u64>,
    failed_blocks: BTreeMap<u64, LaneFailure>,
}

impl ParallelMerkleFinalizerWorker {
    /// Main worker loop that coordinates ingestion, lane completion, and ordered confirmation.
    ///
    /// Flow:
    /// ```text
    /// +------------------- loop -------------------+
    /// | select!                                  |
    /// | 1) recv payload (if inflight < limit)    |
    /// |      -> schedule_block()                 |
    /// | 2) db_staging job finished               |
    /// |      -> mark persisted_ready OR failure  |
    /// | 3) root_compute job finished             |
    /// |      -> mark roots_ready OR failure      |
    /// +--------------------+----------------------+
    ///                      |
    ///                      v
    ///               try_confirm_ready()
    ///                  (strictly next_confirm)
    /// ```
    ///
    /// The loop exits only when the input channel is closed and both job sets are empty.
    async fn run(&mut self, mut receiver: mpsc::Receiver<FinalizedBlockPayload>) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                maybe_payload = receiver.recv(), if Self::can_schedule_more(self.max_inflight_blocks, &self.scheduled_blocks) => {
                    match maybe_payload {
                        Some(payload) => self.schedule_block(payload),
                        None => {
                            if self.db_staging_jobs.is_empty() && self.root_compute_jobs.is_empty() {
                                break;
                            }
                        }
                    }
                }
                Some(joined) = self.db_staging_jobs.join_next(), if !self.db_staging_jobs.is_empty() => {
                    let (block_n, db_staging_result) = joined.context("joining db-staging persistence job")?;
                    match db_staging_result {
                        Ok(()) => {
                            self.persisted_ready.insert(block_n);
                        }
                        Err(error) => {
                            self.record_block_failure(block_n, "db-staging", error);
                        }
                    }
                }
                Some(joined) = self.root_compute_jobs.join_next(), if !self.root_compute_jobs.is_empty() => {
                    let (block_n, root_result) = joined.context("joining root-compute job")?;
                    match root_result {
                        Ok(root_ready) => {
                            self.roots_ready.insert(block_n, root_ready);
                        }
                        Err(error) => {
                            self.record_block_failure(block_n, "root-compute", error);
                        }
                    }
                }
                else => break,
            }

            self.try_confirm_ready().await?;
        }

        // Drain any last ready confirmations after channel close.
        self.try_confirm_ready().await?;
        Ok(())
    }

    /// Schedule both lanes for a single block.
    ///
    /// Flow:
    /// ```text
    /// payload(block_n)
    ///   -> db_staging_jobs.spawn(...)
    ///        writes staged block parts to DB
    ///   -> root_compute_jobs.spawn(...)
    ///        computes state root from snapshot base + cumulative state diff
    /// ```
    ///
    /// A duplicate `block_n` is rejected and recorded as a scheduler failure.
    fn schedule_block(&mut self, payload: FinalizedBlockPayload) {
        let block_n = payload.block_n;
        if !self.scheduled_blocks.insert(block_n) {
            self.record_block_failure(
                block_n,
                "scheduler",
                anyhow::anyhow!("duplicate finalizer payload for block #{block_n}"),
            );
            return;
        }

        let is_boundary = Self::is_boundary_for(self.flush_interval, block_n);

        self.pending_epoch_diffs.insert(block_n, payload.state_diff.clone());
        let cumulative_start = Self::cumulative_range_start(self.snapshot_base_block_n, self.next_confirm);
        let cumulative_state_diff = mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(
            self.pending_epoch_diffs.range(cumulative_start..=block_n).map(|(_, diff)| diff),
        );
        let snapshot_base_block_n = self.snapshot_base_block_n;
        let trie_log_mode = self.trie_log_mode;
        let backend_for_db_staging = Arc::clone(&self.backend);
        let backend_for_root_compute = Arc::clone(&self.backend);

        self.db_staging_jobs.spawn(async move {
            let db_staging_result = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                let mut block = payload.block;
                block.state_diff = payload.state_diff;

                backend_for_db_staging.write_access().write_parallel_merkle_staged_block_data_with_consumed_nonces(
                    &block,
                    &payload.classes,
                    Some(&payload.bouncer_weights),
                    payload.consumed_core_contract_nonces,
                )?;
                Ok(())
            })
            .await
            .context("joining blocking db-staging worker")
            .and_then(|result| result);
            (block_n, db_staging_result)
        });

        self.root_compute_jobs.spawn(async move {
            let root_result = tokio::task::spawn_blocking(move || -> anyhow::Result<RootReady> {
                let computed =
                    backend_for_root_compute.write_access().compute_parallel_merkle_root_from_snapshot_base(
                        snapshot_base_block_n,
                        block_n,
                        &cumulative_state_diff,
                        is_boundary,
                        trie_log_mode,
                    )?;

                Ok(RootReady { root: computed.state_root, overlay: computed.overlay, is_boundary })
            })
            .await
            .context("joining blocking root-compute worker")
            .and_then(|result| result);
            (block_n, root_result)
        });
    }

    /// Confirm all currently-ready blocks in strict sequence (`next_confirm` only).
    ///
    /// Flow per block:
    /// ```text
    /// if failure[next_confirm] -> bail
    /// if persisted_ready && roots_ready for next_confirm:
    ///   confirm staged block with precomputed root
    ///   if boundary:
    ///     flush overlay + checkpoint
    ///     advance snapshot base and drop committed cumulative diffs
    ///   next_confirm += 1
    /// else:
    ///   stop
    /// ```
    ///
    /// This guarantees in-order DB confirmation even if lane completions arrive out-of-order.
    async fn try_confirm_ready(&mut self) -> anyhow::Result<()> {
        if let Some(failure) = Self::next_confirm_failure(self.next_confirm, &self.failed_blocks) {
            anyhow::bail!(
                "parallel finalizer cannot progress at block #{}: {} lane failed: {}",
                self.next_confirm,
                failure.lane,
                failure.message
            );
        }

        loop {
            if !Self::is_confirm_ready(self.next_confirm, &self.persisted_ready, &self.roots_ready) {
                break;
            }
            let Some(root_ready) = self.roots_ready.get(&self.next_confirm).cloned() else {
                break;
            };

            self.backend
                .write_access()
                .confirm_parallel_merkle_staged_block_with_root(
                    self.next_confirm,
                    root_ready.root,
                    /* pre_v0_13_2_hash_override */ true,
                )
                .with_context(|| format!("confirming staged block #{} in parallel finalizer", self.next_confirm))?;

            if root_ready.is_boundary {
                let overlay = root_ready
                    .overlay
                    .as_ref()
                    .with_context(|| format!("missing boundary overlay for block #{}", self.next_confirm))?;

                self.backend
                    .write_access()
                    .flush_parallel_merkle_overlay_and_checkpoint(self.next_confirm, overlay, self.trie_log_mode)
                    .with_context(|| {
                        format!("flushing boundary overlay/checkpoint for block #{}", self.next_confirm)
                    })?;

                self.snapshot_base_block_n = self.next_confirm;
                self.pending_epoch_diffs = self.pending_epoch_diffs.split_off(&(self.next_confirm + 1));
            }

            // Consume readiness only after all DB side effects for this block succeeded.
            self.persisted_ready.remove(&self.next_confirm);
            self.roots_ready.remove(&self.next_confirm);
            self.scheduled_blocks.remove(&self.next_confirm);
            self.next_confirm += 1;
        }
        Ok(())
    }

    fn is_boundary_for(flush_interval: u64, block_n: u64) -> bool {
        (block_n + 1) % flush_interval == 0
    }

    fn cumulative_range_start(snapshot_base_block_n: u64, next_confirm: u64) -> u64 {
        if next_confirm == 0 {
            0
        } else {
            snapshot_base_block_n.saturating_add(1)
        }
    }

    fn is_confirm_ready(
        next_confirm: u64,
        persisted_ready: &BTreeSet<u64>,
        roots_ready: &BTreeMap<u64, RootReady>,
    ) -> bool {
        persisted_ready.contains(&next_confirm) && roots_ready.contains_key(&next_confirm)
    }

    /// Returns whether worker can accept another payload based on inflight block count.
    fn can_schedule_more(max_inflight_blocks: usize, scheduled_blocks: &BTreeSet<u64>) -> bool {
        scheduled_blocks.len() < max_inflight_blocks
    }

    fn next_confirm_failure(next_confirm: u64, failed_blocks: &BTreeMap<u64, LaneFailure>) -> Option<LaneFailure> {
        failed_blocks.get(&next_confirm).cloned()
    }

    fn record_block_failure(&mut self, block_n: u64, lane: &'static str, error: anyhow::Error) {
        let message = format!("{error:#}");
        self.failed_blocks.entry(block_n).or_insert(LaneFailure { lane, message });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(3, 0, false)]
    #[case(3, 1, false)]
    #[case(3, 2, true)]
    #[case(3, 3, false)]
    #[case(3, 4, false)]
    #[case(3, 5, true)]
    fn boundary_rule_respects_flush_interval(
        #[case] flush_interval: u64,
        #[case] block_n: u64,
        #[case] expected: bool,
    ) {
        assert_eq!(ParallelMerkleFinalizerWorker::is_boundary_for(flush_interval, block_n), expected);
    }

    #[rstest]
    #[case(0, 0, 0)]
    #[case(10, 0, 0)]
    #[case(0, 1, 1)]
    #[case(9, 10, 10)]
    fn cumulative_start_cases(#[case] snapshot_base_block_n: u64, #[case] next_confirm: u64, #[case] expected: u64) {
        assert_eq!(
            ParallelMerkleFinalizerWorker::cumulative_range_start(snapshot_base_block_n, next_confirm),
            expected
        );
    }

    #[test]
    fn confirm_ready_requires_both_paths_for_next_block() {
        let mut persisted_ready = BTreeSet::new();
        let mut roots_ready = BTreeMap::new();
        let root_ready = RootReady { root: Felt::ONE, overlay: None, is_boundary: false };

        assert!(!ParallelMerkleFinalizerWorker::is_confirm_ready(5, &persisted_ready, &roots_ready));

        persisted_ready.insert(5);
        assert!(!ParallelMerkleFinalizerWorker::is_confirm_ready(5, &persisted_ready, &roots_ready));

        roots_ready.insert(6, root_ready.clone());
        assert!(!ParallelMerkleFinalizerWorker::is_confirm_ready(5, &persisted_ready, &roots_ready));

        roots_ready.insert(5, root_ready);
        assert!(ParallelMerkleFinalizerWorker::is_confirm_ready(5, &persisted_ready, &roots_ready));
    }

    #[rstest]
    #[case(2, 0, true)]
    #[case(2, 1, true)]
    #[case(2, 2, false)]
    fn can_schedule_more_respects_inflight_bound(
        #[case] max_inflight_blocks: usize,
        #[case] scheduled_count: u64,
        #[case] expected: bool,
    ) {
        let mut scheduled = BTreeSet::new();
        for n in 0..scheduled_count {
            scheduled.insert(10 + n);
        }
        assert_eq!(ParallelMerkleFinalizerWorker::can_schedule_more(max_inflight_blocks, &scheduled), expected);
    }

    #[rstest]
    #[case(6, false)]
    #[case(7, true)]
    fn next_confirm_failure_only_blocks_matching_height(#[case] next_confirm: u64, #[case] should_find: bool) {
        let mut failed_blocks = BTreeMap::new();
        failed_blocks.insert(7, LaneFailure { lane: "root-compute", message: "boom".to_string() });
        let failure = ParallelMerkleFinalizerWorker::next_confirm_failure(next_confirm, &failed_blocks);
        assert_eq!(failure.is_some(), should_find);
        if let Some(failure) = failure {
            assert_eq!(failure.lane, "root-compute");
            assert!(failure.message.contains("boom"));
        }
    }

    #[rstest]
    #[case(Some(10), Some(9), 10)]
    #[case(Some(10), Some(15), 10)]
    #[case(Some(10), None, 10)]
    #[case(None, Some(8), 8)]
    #[case(None, None, 0)]
    fn initial_snapshot_base_cases(
        #[case] latest_confirmed: Option<u64>,
        #[case] latest_checkpoint: Option<u64>,
        #[case] expected: u64,
    ) {
        assert_eq!(initial_snapshot_base(latest_confirmed, latest_checkpoint), expected);
    }
}
