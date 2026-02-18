use crate::{remove_consumed_core_contract_nonces, ParallelMerkleConfig, ParallelMerkleTrieLogMode};
use anyhow::{ensure, Context};
use blockifier::bouncer::BouncerWeights;
use mc_db::{BonsaiOverlay, MadaraBackend, MadaraStorageRead, ParallelMerkleInMemoryTrieLogMode};
use mp_block::FullBlockWithoutCommitments;
use mp_class::ConvertedClass;
use mp_convert::Felt;
use mp_state_update::StateDiff;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
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

const MADARA_PARALLEL_MERKLE_FINALIZER_DEBUG_DELAY_MS: &str = "MADARA_PARALLEL_MERKLE_FINALIZER_DEBUG_DELAY_MS";
const MADARA_PARALLEL_MERKLE_FINALIZER_PATH_A_DEBUG_DELAY_MS: &str =
    "MADARA_PARALLEL_MERKLE_FINALIZER_PATH_A_DEBUG_DELAY_MS";
const MADARA_PARALLEL_MERKLE_FINALIZER_PATH_B_DEBUG_DELAY_MS: &str =
    "MADARA_PARALLEL_MERKLE_FINALIZER_PATH_B_DEBUG_DELAY_MS";

fn parse_delay_from_env(name: &str) -> Option<Duration> {
    std::env::var(name).ok().and_then(|raw| raw.parse::<u64>().ok()).map(Duration::from_millis)
}

fn parallel_merkle_finalizer_debug_delays() -> (Duration, Duration) {
    // Backward compatible fallback: if path-specific envs are absent,
    // keep honoring the old single delay env for both paths.
    let legacy_delay = parse_delay_from_env(MADARA_PARALLEL_MERKLE_FINALIZER_DEBUG_DELAY_MS).unwrap_or(Duration::ZERO);
    let path_a_delay =
        parse_delay_from_env(MADARA_PARALLEL_MERKLE_FINALIZER_PATH_A_DEBUG_DELAY_MS).unwrap_or(legacy_delay);
    let path_b_delay =
        parse_delay_from_env(MADARA_PARALLEL_MERKLE_FINALIZER_PATH_B_DEBUG_DELAY_MS).unwrap_or(legacy_delay);
    (path_a_delay, path_b_delay)
}

fn describe_join_error(error: tokio::task::JoinError) -> String {
    if error.is_panic() {
        let panic_payload = error.into_panic();
        if let Some(message) = panic_payload.downcast_ref::<String>() {
            format!("panic payload (String): {message}")
        } else if let Some(message) = panic_payload.downcast_ref::<&'static str>() {
            format!("panic payload (&'static str): {message}")
        } else {
            format!("panic payload type: {:?}", panic_payload.type_id())
        }
    } else if error.is_cancelled() {
        "task was cancelled".to_string()
    } else {
        format!("join error: {error}")
    }
}

#[derive(Clone)]
pub struct ParallelMerkleFinalizerHandle {
    sender: mpsc::Sender<FinalizedBlockPayload>,
}

impl ParallelMerkleFinalizerHandle {
    pub async fn submit(
        &self,
        payload: FinalizedBlockPayload,
    ) -> Result<(), tokio::sync::mpsc::error::SendError<FinalizedBlockPayload>> {
        self.sender.send(payload).await
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
    let (path_a_debug_delay, path_b_debug_delay) = parallel_merkle_finalizer_debug_delays();
    if !path_a_debug_delay.is_zero() {
        tracing::info!(
            delay_ms = path_a_debug_delay.as_millis(),
            env = MADARA_PARALLEL_MERKLE_FINALIZER_PATH_A_DEBUG_DELAY_MS,
            "🔧 parallel finalizer path-a debug delay enabled"
        );
    }
    if !path_b_debug_delay.is_zero() {
        tracing::info!(
            delay_ms = path_b_debug_delay.as_millis(),
            env = MADARA_PARALLEL_MERKLE_FINALIZER_PATH_B_DEBUG_DELAY_MS,
            "🔧 parallel finalizer path-b debug delay enabled"
        );
    }
    let mut worker = ParallelMerkleFinalizerWorker {
        backend,
        flush_interval: config.flush_interval.max(2),
        trie_log_mode,
        path_a_debug_delay,
        path_b_debug_delay,
        next_confirm,
        snapshot_base_block_n,
        pending_epoch_diffs: BTreeMap::new(),
        persisted_ready: BTreeSet::new(),
        roots_ready: BTreeMap::new(),
        path_a_jobs: JoinSet::new(),
        path_b_jobs: JoinSet::new(),
    };
    worker.initialize_recovery_state().context("recovering parallel merkle finalizer state on startup")?;

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
    path_a_debug_delay: Duration,
    path_b_debug_delay: Duration,
    next_confirm: u64,
    snapshot_base_block_n: u64,
    pending_epoch_diffs: BTreeMap<u64, StateDiff>,
    persisted_ready: BTreeSet<u64>,
    roots_ready: BTreeMap<u64, RootReady>,
    path_a_jobs: JoinSet<anyhow::Result<u64>>,
    path_b_jobs: JoinSet<anyhow::Result<(u64, RootReady)>>,
}

impl ParallelMerkleFinalizerWorker {
    fn initialize_recovery_state(&mut self) -> anyhow::Result<()> {
        let latest_confirmed = self.backend.latest_confirmed_block_n();
        let latest_checkpoint = self
            .backend
            .get_parallel_merkle_latest_checkpoint()
            .context("reading latest parallel merkle checkpoint during startup recovery")?;

        self.snapshot_base_block_n = latest_checkpoint.or(latest_confirmed).unwrap_or(0);
        self.next_confirm = latest_confirmed.map(|n| n + 1).unwrap_or(0);

        if let Some(tip) = latest_confirmed {
            if self.snapshot_base_block_n > tip {
                tracing::warn!(
                    checkpoint = self.snapshot_base_block_n,
                    tip,
                    "parallel merkle checkpoint ahead of confirmed tip, clamping to confirmed tip"
                );
                self.snapshot_base_block_n = tip;
            }
        }

        self.preload_confirmed_epoch_diffs(latest_confirmed)?;
        self.recover_staged_blocks()?;
        Ok(())
    }

    fn preload_confirmed_epoch_diffs(&mut self, latest_confirmed: Option<u64>) -> anyhow::Result<()> {
        let Some(latest_confirmed) = latest_confirmed else {
            return Ok(());
        };

        let start = self.snapshot_base_block_n.saturating_add(1);
        if start > latest_confirmed {
            return Ok(());
        }

        for block_n in start..=latest_confirmed {
            let state_diff = self
                .backend
                .db
                .get_block_state_diff(block_n)?
                .with_context(|| format!("missing confirmed state diff for block #{block_n}"))?;
            self.pending_epoch_diffs.insert(block_n, state_diff);
        }
        Ok(())
    }

    fn recover_staged_blocks(&mut self) -> anyhow::Result<()> {
        let mut staged_blocks =
            self.backend.get_parallel_merkle_staged_blocks().context("listing staged blocks for startup recovery")?;
        staged_blocks.sort_unstable();

        if staged_blocks.is_empty() {
            return Ok(());
        }

        tracing::info!(
            next_confirm = self.next_confirm,
            snapshot_base = self.snapshot_base_block_n,
            count = staged_blocks.len(),
            "parallel merkle startup recovery detected staged blocks"
        );

        for block_n in staged_blocks {
            ensure!(
                block_n == self.next_confirm,
                "staged blocks must be contiguous from next confirm. expected #{}, got #{block_n}",
                self.next_confirm
            );

            self.recover_staged_block(block_n)?;
            self.next_confirm += 1;
        }

        Ok(())
    }

    fn recover_staged_block(&mut self, block_n: u64) -> anyhow::Result<()> {
        let state_diff = self
            .backend
            .db
            .get_block_state_diff(block_n)?
            .with_context(|| format!("missing staged state diff for block #{block_n}"))?;
        self.pending_epoch_diffs.insert(block_n, state_diff);

        let cumulative_start = Self::cumulative_range_start(self.snapshot_base_block_n, self.next_confirm);
        let cumulative_state_diff = mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(
            self.pending_epoch_diffs.range(cumulative_start..=block_n).map(|(_, diff)| diff),
        );
        let is_boundary = Self::is_boundary_for(self.flush_interval, block_n);

        let root_result = self.backend.write_access().compute_parallel_merkle_root_from_snapshot_base(
            self.snapshot_base_block_n,
            block_n,
            &cumulative_state_diff,
            is_boundary,
            self.trie_log_mode,
        )?;

        self.backend
            .write_access()
            .confirm_parallel_merkle_staged_block_with_root(
                block_n,
                root_result.state_root,
                /* pre_v0_13_2_hash_override */ true,
            )
            .with_context(|| format!("confirming staged block #{} during startup recovery", block_n))?;

        if is_boundary {
            let overlay = root_result
                .overlay
                .as_ref()
                .with_context(|| format!("missing boundary overlay for recovered block #{block_n}"))?;
            self.backend
                .write_access()
                .flush_parallel_merkle_overlay_and_checkpoint(block_n, overlay, self.trie_log_mode)
                .with_context(|| format!("flushing checkpoint for recovered boundary block #{}", block_n))?;

            self.snapshot_base_block_n = block_n;
            self.pending_epoch_diffs = self.pending_epoch_diffs.split_off(&(block_n + 1));
        }

        tracing::info!(block_n, is_boundary, "recovered staged block");
        Ok(())
    }

    async fn run(&mut self, mut receiver: mpsc::Receiver<FinalizedBlockPayload>) -> anyhow::Result<()> {
        // Avoid busy-looping once the sender side is dropped. When the channel closes we keep
        // polling in-flight jobs until completion.
        let mut receiver_closed = false;
        loop {
            tokio::select! {
                maybe_payload = receiver.recv(), if !receiver_closed => {
                    match maybe_payload {
                        Some(payload) => {
                            tracing::debug!(
                                block_n = payload.block_n,
                                next_confirm = self.next_confirm,
                                path_a_running = self.path_a_jobs.len(),
                                path_b_running = self.path_b_jobs.len(),
                                "📥 parallel finalizer received payload"
                            );
                            self.schedule_block(payload)
                        }
                        None => {
                            receiver_closed = true;
                        }
                    }
                }
                Some(joined) = self.path_a_jobs.join_next(), if !self.path_a_jobs.is_empty() => {
                    match joined {
                        Ok(Ok(block_n)) => {
                            tracing::debug!(
                                block_n,
                                path_a_running = self.path_a_jobs.len(),
                                path_b_running = self.path_b_jobs.len(),
                                persisted_ready = self.persisted_ready.len(),
                                roots_ready = self.roots_ready.len(),
                                "✅ parallel finalizer path-a job completed"
                            );
                            self.persisted_ready.insert(block_n);
                        }
                        Ok(Err(error)) => {
                            return Err(error).context("parallel finalizer path-a job error");
                        }
                        Err(error) => {
                            let details = describe_join_error(error);
                            return Err(anyhow::anyhow!(details)).context("parallel finalizer path-a job join error");
                        }
                    }
                }
                Some(joined) = self.path_b_jobs.join_next(), if !self.path_b_jobs.is_empty() => {
                    match joined {
                        Ok(Ok((block_n, root_ready))) => {
                            tracing::debug!(
                                block_n,
                                is_boundary = root_ready.is_boundary,
                                path_a_running = self.path_a_jobs.len(),
                                path_b_running = self.path_b_jobs.len(),
                                persisted_ready = self.persisted_ready.len(),
                                roots_ready = self.roots_ready.len(),
                                "✅ parallel finalizer path-b job completed"
                            );
                            self.roots_ready.insert(block_n, root_ready);
                        }
                        Ok(Err(error)) => {
                            return Err(error).context("parallel finalizer path-b job error");
                        }
                        Err(error) => {
                            let details = describe_join_error(error);
                            return Err(anyhow::anyhow!(details)).context("parallel finalizer path-b job join error");
                        }
                    }
                }
                else => break,
            }

            self.try_confirm_ready().await?;

            if receiver_closed && self.path_a_jobs.is_empty() && self.path_b_jobs.is_empty() {
                break;
            }
        }

        // Drain any last ready confirmations after channel close.
        self.try_confirm_ready().await?;
        Ok(())
    }

    fn schedule_block(&mut self, payload: FinalizedBlockPayload) {
        let block_n = payload.block_n;
        if block_n < self.next_confirm {
            tracing::warn!(block_n, next_confirm = self.next_confirm, "dropping already-confirmed payload");
            return;
        }
        let is_boundary = Self::is_boundary_for(self.flush_interval, block_n);
        let schedule_started = Instant::now();
        tracing::debug!(
            block_n,
            next_confirm = self.next_confirm,
            is_boundary,
            pending_epoch_diff_count = self.pending_epoch_diffs.len(),
            path_a_running = self.path_a_jobs.len(),
            path_b_running = self.path_b_jobs.len(),
            "🧩 scheduling block for parallel finalizer pipelines"
        );
        let in_flight_jobs = self.path_a_jobs.len() + self.path_b_jobs.len();
        if in_flight_jobs > 0 {
            tracing::debug!(
                block_n,
                in_flight_jobs,
                path_a_running = self.path_a_jobs.len(),
                path_b_running = self.path_b_jobs.len(),
                "🧵 detected across-block overlap opportunity for parallel finalizer"
            );
        }
        self.pending_epoch_diffs.insert(block_n, payload.state_diff.clone());
        let in_flight_blocks = self.pending_epoch_diffs.len();
        let active_jobs = self.path_a_jobs.len() + self.path_b_jobs.len();
        tracing::debug!(
            block_n,
            is_boundary,
            in_flight_blocks,
            active_jobs,
            "🧩 block queued for parallel finalizer (across-block overlap indicator)"
        );
        let cumulative_start = Self::cumulative_range_start(self.snapshot_base_block_n, self.next_confirm);
        let cumulative_state_diff = mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(
            self.pending_epoch_diffs.range(cumulative_start..=block_n).map(|(_, diff)| diff),
        );
        let snapshot_base_block_n = self.snapshot_base_block_n;
        let trie_log_mode = self.trie_log_mode;
        let backend_for_path_a = Arc::clone(&self.backend);
        let backend_for_path_b = Arc::clone(&self.backend);
        let path_a_debug_delay = self.path_a_debug_delay;
        let block_n_for_paths = block_n;
        // Keep bootstrap blocks comparable with sequential mode when debug delay is enabled.
        // We only inject Path-B delay after the first epoch boundary so the initial setup
        // transactions (fund/declare/deploy in e2e flows) are not shifted into different blocks.
        let path_b_debug_delay = if self.path_b_debug_delay.is_zero() || block_n_for_paths <= self.flush_interval {
            Duration::ZERO
        } else {
            self.path_b_debug_delay
        };

        self.path_a_jobs.spawn(async move {
            let started = Instant::now();

            tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
                let thread_id = std::thread::current().id();
                tracing::debug!(
                    block_n = block_n_for_paths,
                    thread_id = ?thread_id,
                    "🧪 finalizer path-a worker started"
                );

                if !path_a_debug_delay.is_zero() {
                    tracing::debug!(
                        block_n = block_n_for_paths,
                        thread_id = ?thread_id,
                        delay_ms = path_a_debug_delay.as_millis(),
                        "⏱️ finalizer path-a debug delay"
                    );
                    std::thread::sleep(path_a_debug_delay);
                }

                let mut block = payload.block;
                block.state_diff = payload.state_diff;

                remove_consumed_core_contract_nonces(
                    backend_for_path_a.as_ref(),
                    payload.consumed_core_contract_nonces,
                    "parallel finalizer",
                )?;

                backend_for_path_a.write_access().write_parallel_merkle_staged_block_data(
                    &block,
                    &payload.classes,
                    Some(&payload.bouncer_weights),
                )?;
                tracing::debug!(
                    block_n = block_n_for_paths,
                    thread_id = ?thread_id,
                    elapsed_ms = started.elapsed().as_millis(),
                    "✅ finalizer path-a worker done"
                );
                Ok(block_n)
            })
            .await
            .context("joining blocking path-a worker")?
        });

        self.path_b_jobs.spawn(async move {
            let started = Instant::now();

            tokio::task::spawn_blocking(move || -> anyhow::Result<(u64, RootReady)> {
                let thread_id = std::thread::current().id();
                tracing::debug!(
                    block_n,
                    thread_id = ?thread_id,
                    "🧪 finalizer path-b worker started"
                );

                if !path_b_debug_delay.is_zero() {
                    tracing::debug!(
                        block_n,
                        thread_id = ?thread_id,
                        delay_ms = path_b_debug_delay.as_millis(),
                        "⏱️ finalizer path-b debug delay"
                    );
                    std::thread::sleep(path_b_debug_delay);
                }

                let root_result = backend_for_path_b.write_access().compute_parallel_merkle_root_from_snapshot_base(
                    snapshot_base_block_n,
                    block_n,
                    &cumulative_state_diff,
                    is_boundary,
                    trie_log_mode,
                )?;

                tracing::debug!(
                    block_n,
                    thread_id = ?thread_id,
                    elapsed_ms = started.elapsed().as_millis(),
                    "✅ finalizer path-b worker done"
                );
                Ok((block_n, RootReady { root: root_result.state_root, overlay: root_result.overlay, is_boundary }))
            })
            .await
            .context("joining blocking path-b worker")?
        });

        tracing::debug!(
            block_n = block_n,
            schedule_ms = schedule_started.elapsed().as_millis(),
            "✅ scheduled parallel finalizer path-a/path-b jobs"
        );
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

            let confirm_result = self
                .backend
                .write_access()
                .confirm_parallel_merkle_staged_block_with_root(
                    self.next_confirm,
                    root_ready.root,
                    /* pre_v0_13_2_hash_override */ true,
                )
                .with_context(|| format!("confirming staged block #{} in parallel finalizer", self.next_confirm))?;

            let state_diff = self.pending_epoch_diffs.get(&self.next_confirm);
            let (storage_diffs, nonce_updates, deployed_contracts, declared_classes) = state_diff
                .map(|sd| {
                    (
                        sd.storage_diffs.iter().map(|item| item.storage_entries.len()).sum::<usize>(),
                        sd.nonces.len(),
                        sd.deployed_contracts.len(),
                        sd.declared_classes.len() + sd.old_declared_contracts.len(),
                    )
                })
                .unwrap_or((0, 0, 0, 0));

            tracing::info!(
                "✅ Confirmed block #{} via parallel finalizer — state_root: {:#x}, block_hash: {:#x}, \
                 state_diff: {{ storage_diffs: {}, nonce_updates: {}, deployed: {}, declared: {} }}",
                self.next_confirm,
                confirm_result.new_state_root,
                confirm_result.block_hash,
                storage_diffs,
                nonce_updates,
                deployed_contracts,
                declared_classes,
            );

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
    use mc_db::MadaraBackend;
    use mp_block::{header::PreconfirmedHeader, FullBlockWithoutCommitments, TransactionWithReceipt};
    use mp_chain_config::ChainConfig;
    use mp_receipt::{Event, EventWithTransactionHash, InvokeTransactionReceipt};
    use mp_state_update::StateDiff;
    use mp_transactions::InvokeTransactionV0;

    fn make_staged_block(block_n: u64, tx_hash: Felt) -> FullBlockWithoutCommitments {
        let header = PreconfirmedHeader { block_number: block_n, ..Default::default() };
        let tx = TransactionWithReceipt {
            transaction: InvokeTransactionV0::default().into(),
            receipt: InvokeTransactionReceipt { transaction_hash: tx_hash, ..Default::default() }.into(),
        };
        let events = vec![EventWithTransactionHash {
            transaction_hash: tx_hash,
            event: Event {
                from_address: Felt::from(123_u64),
                keys: vec![Felt::from(1_u64)],
                data: vec![Felt::from(2_u64)],
            },
        }];

        FullBlockWithoutCommitments { header, state_diff: StateDiff::default(), transactions: vec![tx], events }
    }

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

    #[tokio::test]
    async fn startup_recovery_confirms_staged_blocks_in_order() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let block_0 = make_staged_block(0, Felt::from_hex_unchecked("0xabc"));
        let block_1 = make_staged_block(1, Felt::from_hex_unchecked("0xabd"));

        backend.write_access().write_parallel_merkle_staged_block_data(&block_0, &[], None).unwrap();
        backend.write_access().write_parallel_merkle_staged_block_data(&block_1, &[], None).unwrap();
        assert_eq!(backend.latest_confirmed_block_n(), None);

        let _handle = spawn_parallel_merkle_finalizer(
            backend.clone(),
            ParallelMerkleConfig {
                enabled: true,
                flush_interval: 3,
                max_inflight: 8,
                trie_log_mode: ParallelMerkleTrieLogMode::Checkpoint,
            },
        )
        .expect("finalizer startup recovery should succeed");

        assert_eq!(backend.latest_confirmed_block_n(), Some(1));
        assert!(!backend.has_parallel_merkle_staged_block(0).unwrap());
        assert!(!backend.has_parallel_merkle_staged_block(1).unwrap());
    }

    #[tokio::test]
    async fn startup_recovery_rejects_non_contiguous_staged_blocks() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));
        let block_1 = make_staged_block(1, Felt::from_hex_unchecked("0xabc"));

        backend.write_access().write_parallel_merkle_staged_block_data(&block_1, &[], None).unwrap();

        let err = spawn_parallel_merkle_finalizer(
            backend,
            ParallelMerkleConfig {
                enabled: true,
                flush_interval: 3,
                max_inflight: 8,
                trie_log_mode: ParallelMerkleTrieLogMode::Checkpoint,
            },
        )
        .err()
        .expect("non-contiguous staged blocks should fail startup recovery");

        let err_text = format!("{err:#}");
        assert!(err_text.contains("expected #0, got #1"), "unexpected startup recovery error: {err_text}");
    }
}
