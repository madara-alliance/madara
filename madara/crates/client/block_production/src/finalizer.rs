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
    let snapshot_base_block_n = backend
        .get_parallel_merkle_latest_checkpoint()
        .context("reading parallel merkle latest checkpoint")?
        .or(latest_confirmed)
        .unwrap_or(0);

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
        path_a_jobs: JoinSet::new(),
        path_b_jobs: JoinSet::new(),
    };
    tokio::spawn(async move {
        if let Err(error) = worker.run(receiver).await {
            tracing::error!(%error, "parallel merkle finalizer worker terminated with error");
        }
    });

    Ok(ParallelMerkleFinalizerHandle { sender })
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
    path_a_jobs: JoinSet<anyhow::Result<u64>>,
    path_b_jobs: JoinSet<anyhow::Result<(u64, RootReady)>>,
}

impl ParallelMerkleFinalizerWorker {
    async fn run(&mut self, mut receiver: mpsc::Receiver<FinalizedBlockPayload>) -> anyhow::Result<()> {
        loop {
            tokio::select! {
                maybe_payload = receiver.recv() => {
                    match maybe_payload {
                        Some(payload) => self.schedule_block(payload),
                        None => {
                            if self.path_a_jobs.is_empty() && self.path_b_jobs.is_empty() {
                                break;
                            }
                        }
                    }
                }
                Some(joined) = self.path_a_jobs.join_next(), if !self.path_a_jobs.is_empty() => {
                    let block_n = joined.context("joining path-a persistence job")??;
                    self.persisted_ready.insert(block_n);
                }
                Some(joined) = self.path_b_jobs.join_next(), if !self.path_b_jobs.is_empty() => {
                    let (block_n, root_ready) = joined.context("joining path-b root job")??;
                    self.roots_ready.insert(block_n, root_ready);
                }
                else => break,
            }

            self.try_confirm_ready().await?;
        }

        // Drain any last ready confirmations after channel close.
        self.try_confirm_ready().await?;
        Ok(())
    }

    fn schedule_block(&mut self, payload: FinalizedBlockPayload) {
        let block_n = payload.block_n;
        let is_boundary = Self::is_boundary_for(self.flush_interval, block_n);

        self.pending_epoch_diffs.insert(block_n, payload.state_diff.clone());
        let cumulative_start = Self::cumulative_range_start(self.snapshot_base_block_n, self.next_confirm);
        let cumulative_state_diff = mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(
            self.pending_epoch_diffs.range(cumulative_start..=block_n).map(|(_, diff)| diff),
        );
        let snapshot_base_block_n = self.snapshot_base_block_n;
        let trie_log_mode = self.trie_log_mode;
        let backend_for_path_a = Arc::clone(&self.backend);
        let backend_for_path_b = Arc::clone(&self.backend);

        self.path_a_jobs.spawn(async move {
            tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
                let mut block = payload.block;
                block.state_diff = payload.state_diff;

                for l1_nonce in payload.consumed_core_contract_nonces {
                    backend_for_path_a
                        .remove_pending_message_to_l2(l1_nonce)
                        .context("removing consumed l1->l2 nonce in parallel finalizer")?;
                }

                backend_for_path_a.write_access().write_parallel_merkle_staged_block_data(
                    &block,
                    &payload.classes,
                    Some(&payload.bouncer_weights),
                )?;
                Ok(block_n)
            })
            .await
            .context("joining blocking path-a worker")?
        });

        self.path_b_jobs.spawn(async move {
            tokio::task::spawn_blocking(move || -> anyhow::Result<(u64, RootReady)> {
                let root_result = backend_for_path_b.write_access().compute_parallel_merkle_root_from_snapshot_base(
                    snapshot_base_block_n,
                    block_n,
                    &cumulative_state_diff,
                    is_boundary,
                    trie_log_mode,
                )?;

                Ok((block_n, RootReady { root: root_result.state_root, overlay: root_result.overlay, is_boundary }))
            })
            .await
            .context("joining blocking path-b worker")?
        });
    }

    async fn try_confirm_ready(&mut self) -> anyhow::Result<()> {
        loop {
            if !Self::is_confirm_ready(self.next_confirm, &self.persisted_ready, &self.roots_ready) {
                break;
            }
            let Some(root_ready) = self.roots_ready.get(&self.next_confirm).cloned() else {
                break;
            };

            self.persisted_ready.remove(&self.next_confirm);
            self.roots_ready.remove(&self.next_confirm);

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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_rule_respects_flush_interval() {
        assert!(!ParallelMerkleFinalizerWorker::is_boundary_for(3, 0));
        assert!(!ParallelMerkleFinalizerWorker::is_boundary_for(3, 1));
        assert!(ParallelMerkleFinalizerWorker::is_boundary_for(3, 2));
        assert!(!ParallelMerkleFinalizerWorker::is_boundary_for(3, 3));
        assert!(!ParallelMerkleFinalizerWorker::is_boundary_for(3, 4));
        assert!(ParallelMerkleFinalizerWorker::is_boundary_for(3, 5));
    }

    #[test]
    fn cumulative_start_includes_block_zero_without_confirmed_tip() {
        assert_eq!(ParallelMerkleFinalizerWorker::cumulative_range_start(0, 0), 0);
        assert_eq!(ParallelMerkleFinalizerWorker::cumulative_range_start(10, 0), 0);
    }

    #[test]
    fn cumulative_start_skips_persisted_snapshot_base_after_confirmed_tip() {
        assert_eq!(ParallelMerkleFinalizerWorker::cumulative_range_start(0, 1), 1);
        assert_eq!(ParallelMerkleFinalizerWorker::cumulative_range_start(9, 10), 10);
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
}
