//! Madara block production. This crate is responsible for _producing state_ when the node is
//! running as a sequencer. Full node sync does _not_ use this crate. Instead, refer to [`mc_sync`].
//!
//! # Execution Model
//!
//! Block production works by processing transactions which are streamed to it from the [mempool].
//! For performance reasons, this is separated into a **batching phase**, an **aggregation phase**
//! and a **pending phase**, which are optimistically parallelized. Effectively, this allows block
//! production to lag behind on certain tasks (on large blocks for example) while still being able
//! to make progress wherever it can.
//!
//! ## Batching Phase
//!
//! Because efficient block production is complicated (help my head hurts), transaction aggregation
//! is handled by a [`Batcher`] which consumes three transaction streams (this has the disadvantage
//! of making the code quite hard to follow at times :/ ).
//!
//! - The `l1_tx_stream` sends all l1 transactions received as part of l1 sync (this is different
//!   from l2 sync, l1 sync is always active even during block production because we need a way to
//!   register l1 to l2 messages even as we are producing new blocks).
//!
//! - The `mempool_tx_stream` sends all validated transaction which have been added to the mempool,
//!   following whichever mempool ordering policy is in use at the time of block production (this
//!   can be first-come-first-served or fee-market).
//!
//! - The `bypass_txs_stream` allows transactions to be added to block production _without having
//!   them be validated by the mempool_. This is used by certain _permisioned_ (admin) endpoints and
//!   is useful when initially setting up a chain, where trying to deploy some genesis contracts
//!   might result in an invalidation until they have been deployed. This kind of cyclical
//!   invalidation requires a way to force-add transactions, and the bypass stream does just that.
//!
//! The batcher aggregates transactions from all these streams into a 'batch', which it sends back
//! to the main block production task via message passing using channels, where the sending end
//! is [`ExecutorThreadHandle::send_batch`] and the receiving end is
//! [`ExecutorThread::incoming_batches`].
//!
//! ## Aggregation Phase
//!
//! Between transaction batches, updates to the block production state is handled by the
//! [`BlockProductionTask`], which is responsible for starting, running and monitoring block
//! production. The aggregation phase is used to drive updates to the block production state through
//! an actor model implemented via message passing, where the block production task drives itself to
//! completion by messaging itself across threads, state updates and method calls. This is handled
//! by [`process_reply`], which needs to handle the following messages:
//!
//! - [`StartNewBlock`]: this message is sent whenever the [`ExecutorThread`] starts a new block and
//!   it instructs the [`BlockProductionTask`] to clear its [`PendingBlockState`] in preparation for
//!   the next batch.
//!
//! - [`BatchExecuted`]: this message is sent whenever the [`ExecutorThread`] has finished executing
//!   a batch, marking it as ready to consume by the [`BlockProductionTask`]. When this is received,
//!   the latest batch is added to the [`PendingBlockState`].
//!
//! - [`EndBlock`]: this message is sent by the [`ExecutorThread`] under one of several condition.
//!   Either the block has been forcefully closed (for example by an admin endpoint), or it is full
//!   as per the constraints set it the chain config, else the block time has elapsed as per the
//!   constraints set in the chain config. In any of these cases, whenever the [`ExecutorThread`]
//!   receives this message it will proceed to finalize (seal) the pending block and store it to db
//!   as a full block.
//!
//! - [`EndFinalBlock`]: sent during graceful shutdown when batch channel closes. Contains
//!   `Some(summary)` to close an existing block, or `None` to signal completion without a block.
//!
//! ## Pending Phase
//!
//! The [`PendingBlockState`] is primarily kept in RAM but is also flushed to the database as a
//! **pre-confirmed block**. This ensures that if the node crashes or restarts during block
//! production, we can recover the work done so far.
//!
//! Currently, we flush the pending block state to the database whenever a new batch of transactions
//! is executed (`BatchExecuted` message). This persistence allows us to recover the pre-confirmed
//! block upon restart.
//!
//! ### Restart Recovery
//!
//! When Madara starts, it checks for an existing pre-confirmed block. If found:
//! 1. It loads the saved **runtime execution configuration** (ChainConfig, VersionedConstants, etc.)
//!    to ensure re-execution uses the exact same parameters as the original execution.
//! 2. It **re-executes** all transactions in the pre-confirmed block. This is necessary because
//!    intermediate execution artifacts (like bouncer weights and state diffs) are not fully persisted.
//! 3. It closes the block immediately, effectively resuming the chain from where it left off.
//!
//! This mechanism guarantees consistency (e.g., transaction receipts match exactly) even if the
//! node's configuration changes between restarts (e.g., toggling fee charging).
//!
//! ## Graceful Shutdown and Error Handling
//!
//! The [`BlockProductionTask::run`] method implements graceful shutdown and error handling for
//! batcher and executor tasks. The main loop tracks completion of both tasks, which only complete
//! during shutdown scenarios (cancellation, error, or panic).
//!
//! ### Graceful Shutdown
//!
//! When a cancellation signal is received:
//! 1. The batcher detects cancellation and exits gracefully, closing the `send_batch` channel
//! 2. The executor detects the channel closure and finalizes any open block
//! 3. The executor sends an `EndFinalBlock` message (shutdown-specific) and then completes
//! 4. The main loop processes the `EndFinalBlock`, closes the block, and exits when both tasks complete
//!
//! ### Batcher Panic/Error
//!
//! If the batcher encounters an error or panics:
//! - **With preconfirmed block**: The error is saved and graceful shutdown is attempted. The batcher
//!   closes the channel, executor closes the block, and shutdown completes with the saved error.
//! - **Without preconfirmed block**: The error is returned immediately (no need to wait for executor).
//!
//! ### Executor Panic
//!
//! If the executor thread panics:
//! - The panic is caught and propagated via the `stop` channel
//! - The main loop resumes the panic, causing the block to remain preconfirmed
//! - The preconfirmed block will be handled on restart
//!
//! The loop exits when:
//! - Batcher completed AND `EndFinalBlock` was processed → returns `Ok(())` or saved batcher error
//!
//! [mempool]: mc_mempool
//! [`StartNewBlock`]: ExecutorMessage::StartNewBlock
//! [`BatchExecuted`]: ExecutorMessage::BatchExecuted
//! [`EndBlock`]: ExecutorMessage::EndBlock
//! [`EndFinalBlock`]: ExecutorMessage::EndFinalBlock
//! [`ExecutorThreadHandle::send_batch`]: executor::ExecutorThreadHandle::send_batch
//! [`ExecutorThread::incoming_batches`]: executor::thread::ExecutorThread::incoming_batches
//! [`ExecutorThread`]: executor::thread::ExecutorThread
//! [`process_reply`]: BlockProductionTask::process_reply

use crate::batcher::Batcher;
use crate::close_queue::{CloseJobCompletion, QueuedClosePayload};
use crate::finalizer::FinalizerHandle;
use crate::metrics::BlockProductionMetrics;
use crate::util::BlockExecutionContext;
use anyhow::Context;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use executor::{BatchExecutionResult, ExecutorMessage};
use mc_db::close_pipeline_contract::ClosePreconfirmedResult;
use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use mc_db::{MadaraBackend, MadaraPreconfirmedBlockView, MadaraStateView};
use mc_exec::execution::TxInfo;
use mc_exec::LayeredStateAdapter;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_block::TransactionWithReceipt;
use mp_chain_config::RuntimeExecutionConfig;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::StateDiff;
use mp_state_update::{ClassUpdateItem, DeclaredClassCompiledClass, TransactionStateUpdate};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::TransactionWithHash;
use mp_utils::rayon::global_spawn_rayon_task;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use std::collections::{HashSet, VecDeque};
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::sync::watch;

mod batcher;
mod close_queue;
mod executor;
mod finalizer;
mod handle;
pub mod metrics;
mod util;

pub use handle::BlockProductionHandle;

const PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT: usize = 10;
// TODO(mohit): demote the high-volume replay/merkle instrumentation below once the current bottleneck investigation is complete.
static PARALLEL_ROOT_ACTIVE_JOBS: AtomicUsize = AtomicUsize::new(0);

fn active_parallel_root_jobs() -> usize {
    PARALLEL_ROOT_ACTIVE_JOBS.load(Ordering::Relaxed)
}

struct ParallelRootJobGuard;

impl ParallelRootJobGuard {
    fn acquire() -> (Self, usize) {
        let active_jobs = PARALLEL_ROOT_ACTIVE_JOBS.fetch_add(1, Ordering::Relaxed) + 1;
        (Self, active_jobs)
    }
}

impl Drop for ParallelRootJobGuard {
    fn drop(&mut self) {
        PARALLEL_ROOT_ACTIVE_JOBS.fetch_sub(1, Ordering::Relaxed);
    }
}

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

struct ParallelComputedClosePayload {
    payload: QueuedClosePayload,
    root_response: mc_db::rocksdb::global_trie::in_memory::InMemoryRootComputation,
    parallel_summary: ParallelMerkleSummary,
}

fn validate_parallel_queue_invariant(parallel_merkle_enabled: bool, close_queue_capacity: usize) -> anyhow::Result<()> {
    if parallel_merkle_enabled && close_queue_capacity < PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT {
        anyhow::bail!(
            "QueueInvariantViolated: configured capacity {} is below protocol minimum {}",
            close_queue_capacity,
            PARALLEL_MERKLE_PROTOCOL_MIN_INFLIGHT
        );
    }
    Ok(())
}

fn prune_diffs_since_snapshot(diffs_since_snapshot: &mut Vec<(u64, StateDiff)>, completed_block_n: u64) {
    diffs_since_snapshot.retain(|(n, _)| *n > completed_block_n);
}

fn collect_diffs_for_root_from_base(
    diffs_since_snapshot: &[(u64, StateDiff)],
    base_block_n: Option<u64>,
    target_block_n: u64,
) -> anyhow::Result<Vec<StateDiff>> {
    let mut expected_block_n = base_block_n.map_or(0, |block_n| block_n.saturating_add(1));
    let mut collected = Vec::new();

    for (block_n, state_diff) in diffs_since_snapshot.iter().filter(|(block_n, _)| *block_n <= target_block_n) {
        if *block_n < expected_block_n {
            continue;
        }
        anyhow::ensure!(
            *block_n == expected_block_n,
            "Missing tracked state diff for block #{expected_block_n} while preparing root for block #{target_block_n} from base {base_block_n:?}"
        );
        collected.push(state_diff.clone());
        expected_block_n = expected_block_n.saturating_add(1);
    }

    anyhow::ensure!(
        expected_block_n == target_block_n.saturating_add(1),
        "Incomplete tracked state diffs for root of block #{target_block_n} from base {base_block_n:?}"
    );

    Ok(collected)
}

fn saturating_u128_to_u64(metric_name: &str, value: u128) -> u64 {
    value.try_into().unwrap_or_else(|_| {
        tracing::warn!("{metric_name} ({value}) exceeds u64::MAX ({}), saturating to u64::MAX for metrics", u64::MAX);
        u64::MAX
    })
}

fn record_closed_block_summary_metrics(
    metrics: &BlockProductionMetrics,
    block_number: u64,
    tx_count: u64,
    execution_duration: Duration,
    close_commit_stage_duration: Duration,
    close_end_to_end_duration: Duration,
    close_post_execution_duration: Option<Duration>,
    block_production_time: Duration,
    exec_stats: &util::ExecutionStats,
) {
    let l2_gas_consumed_u64 = saturating_u128_to_u64("block_l2_gas_consumed", exec_stats.l2_gas_consumed);

    metrics.block_execution_duration.record(execution_duration.as_secs_f64(), &[]);
    metrics.block_execution_last.record(execution_duration.as_secs_f64(), &[]);
    metrics.close_end_to_end_duration.record(close_end_to_end_duration.as_secs_f64(), &[]);
    metrics.close_end_to_end_last.record(close_end_to_end_duration.as_secs_f64(), &[]);
    if let Some(duration) = close_post_execution_duration {
        metrics.close_post_execution_duration.record(duration.as_secs_f64(), &[]);
        metrics.close_post_execution_last.record(duration.as_secs_f64(), &[]);
    }
    metrics.close_commit_stage_duration.record(close_commit_stage_duration.as_secs_f64(), &[]);
    metrics.close_commit_stage_last.record(close_commit_stage_duration.as_secs_f64(), &[]);

    // Compatibility metrics retained for existing dashboards/alerts. These now clearly represent
    // the commit-stage duration, not end-to-end close latency.
    metrics.close_block_total_duration.record(close_commit_stage_duration.as_secs_f64(), &[]);
    metrics.close_block_total_last.record(close_commit_stage_duration.as_secs_f64(), &[]);
    metrics.block_close_time.record(close_commit_stage_duration.as_secs_f64(), &[]);
    metrics.block_close_time_last.record(close_commit_stage_duration.as_secs_f64(), &[]);

    metrics.block_counter.add(1, &[]);
    metrics.block_gauge.record(block_number, &[]);
    metrics.transaction_counter.add(tx_count, &[]);
    metrics.block_production_time.record(block_production_time.as_secs_f64(), &[]);
    metrics.block_production_time_last.record(block_production_time.as_secs_f64(), &[]);

    metrics.block_tx_count.record(tx_count, &[]);
    metrics.block_batches_executed_gauge.record(exec_stats.n_batches as u64, &[]);
    metrics.block_txs_added_to_block_gauge.record(exec_stats.n_added_to_block as u64, &[]);
    metrics.block_txs_executed_gauge.record(exec_stats.n_executed as u64, &[]);
    metrics.block_txs_reverted_gauge.record(exec_stats.n_reverted as u64, &[]);
    metrics.block_txs_rejected_gauge.record(exec_stats.n_rejected as u64, &[]);
    metrics.block_l2_gas_consumed_gauge.record(l2_gas_consumed_u64, &[]);
}

/// Used for listening to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock { block_n: u64 },
    BatchExecuted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MempoolIntakeMode {
    Running,
    Paused,
}

#[derive(Debug)]
pub(crate) struct CurrentBlockState {
    backend: Arc<MadaraBackend>,
    pub block_number: u64,
    pub consumed_core_contract_nonces: HashSet<u64>,
    /// We need to keep track of deployed contracts, because blockifier can't make the difference between replaced class / deployed contract :/
    pub deployed_contracts: HashSet<Felt>,
    /// Track when block production started for metrics
    pub block_start_time: Instant,
    /// Accumulated execution stats across all batches for this block
    pub accumulated_stats: util::ExecutionStats,
    /// Timestamp for the last batch that finished execution in the executor thread.
    pub last_execution_finished_at: Option<Instant>,
}

impl CurrentBlockState {
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self {
            backend,
            block_number,
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
            block_start_time: Instant::now(),
            accumulated_stats: Default::default(),
            last_execution_finished_at: None,
        }
    }
    /// Process the execution result, merging it with the current pending state
    pub async fn append_batch(&mut self, mut batch: BatchExecutionResult) -> anyhow::Result<()> {
        let mut executed = vec![];

        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().context("Invalid nonce while appending batch")?);
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                let declared_class = additional_info.declared_class.take().filter(|_| !execution_info.is_reverted());

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);
                let converted_tx = TransactionWithHash::from(blockifier_tx.clone());

                // Extract paid_fee_on_l1 from L1 handler transactions
                let paid_fee_on_l1 = match &blockifier_tx {
                    blockifier::transaction::transaction_execution::Transaction::L1Handler(l1_tx) => {
                        Some(l1_tx.paid_fee_on_l1.0)
                    }
                    _ => None,
                };

                executed.push(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt { transaction: converted_tx.transaction, receipt },
                    state_diff: TransactionStateUpdate {
                        nonces: state_diff
                            .nonces
                            .into_iter()
                            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
                            .collect(),
                        contract_class_hashes: state_diff
                            .class_hashes
                            .into_iter()
                            .map(|(contract_addr, class_hash)| {
                                let entry = if !self.deployed_contracts.contains(&contract_addr)
                                    && !self.backend.view_on_latest_confirmed().is_contract_deployed(&contract_addr)?
                                {
                                    self.deployed_contracts.insert(contract_addr.to_felt());
                                    ClassUpdateItem::DeployedContract(class_hash.to_felt())
                                } else {
                                    ClassUpdateItem::ReplacedClass(class_hash.to_felt())
                                };

                                Ok((contract_addr.to_felt(), entry))
                            })
                            .collect::<anyhow::Result<_>>()?,
                        storage_diffs: state_diff
                            .storage
                            .into_iter()
                            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
                            .collect(),
                        declared_classes: declared_class
                            .iter()
                            .map(|class| {
                                (
                                    *class.class_hash(),
                                    class
                                        .as_sierra()
                                        .and_then(|class| {
                                            // Use canonical hash (v2 if present, else v1)
                                            let hash =
                                                class.info.compiled_class_hash_v2.or(class.info.compiled_class_hash)?;
                                            Some(DeclaredClassCompiledClass::Sierra(hash))
                                        })
                                        .unwrap_or(DeclaredClassCompiledClass::Legacy),
                                )
                            })
                            .collect(),
                    },
                    declared_class,
                    arrived_at: additional_info.arrived_at,
                    paid_fee_on_l1,
                })
            }
        }

        let backend = self.backend.clone();
        global_spawn_rayon_task(move || {
            backend
                .write_access()
                .append_to_preconfirmed(&executed, /* candidates */ [])
                .context("Appending to preconfirmed block")
        })
        .await?;

        let stats = mem::take(&mut batch.stats);
        if stats.n_executed > 0 {
            tracing::debug!(
                txs_executed_in_batch = stats.n_executed,
                txs_added_to_block = stats.n_added_to_block,
                txs_reverted = stats.n_reverted,
                txs_rejected = stats.n_rejected,
                batch_exec_duration_ms = stats.exec_duration.as_secs_f64() * 1000.0,
                "🧮 Executed and added {} transaction(s) to the preconfirmed block at height {} - {:.3?}",
                stats.n_added_to_block,
                self.block_number,
                stats.exec_duration,
            );
            tracing::debug!("Tick stats {:?}", stats);
        }
        Ok(())
    }
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
pub(crate) enum TaskState {
    NotExecuting {
        /// [`None`] when the next block to execute is genesis.
        latest_block_n: Option<u64>,
    },
    Executing(CurrentBlockState),
}

/// The block production task consumes transactions from the mempool in batches.
///
/// This is to allow optimistic concurrency. However, the block may get full during batch execution,
/// and we need to re-add the transactions back into the mempool.
///
/// To understand block production in madara, you should probably start with the [`mp_chain_config::ChainConfig`]
/// documentation.
pub struct BlockProductionTask {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    close_queue_capacity: usize,
    current_state: Option<TaskState>,
    metrics: Arc<BlockProductionMetrics>,
    state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
    handle: BlockProductionHandle,
    executor_commands_recv: Option<mpsc::UnboundedReceiver<executor::ExecutorCommand>>,
    l1_client: Arc<dyn SettlementClient>,
    bypass_tx_input: Option<mpsc::Receiver<ValidatedTransaction>>,
    mempool_intake_rx: watch::Receiver<MempoolIntakeMode>,
    no_charge_fee: bool,
    replay_mode_enabled: bool,
    parallel_merkle_enabled: bool,
    parallel_merkle_compare_sequential: bool,
    parallel_merkle_root_workers: usize,
    parallel_merkle_flush_interval: u64,
    parallel_merkle_trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode,
    diffs_since_snapshot: Vec<(u64, StateDiff)>,
    pending_completions: VecDeque<(u64, tokio::sync::oneshot::Receiver<anyhow::Result<CloseJobCompletion>>)>,
}

impl BlockProductionTask {
    /// Creates a new BlockProductionTask.
    ///
    /// # Parameters
    ///
    /// * `mempool_paused`: If true, block production starts with mempool intake paused.
    /// * `no_charge_fee`: Determines whether fees are charged during transaction execution.
    ///
    /// # TODO(mohit 18/11/2025): Update the code to use config same as pre-close
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        mempool_paused: bool,
        no_charge_fee: bool,
    ) -> Self {
        let (sender, recv) = mpsc::unbounded_channel();
        let (bypass_input_sender, bypass_tx_input) = mpsc::channel(1024);
        let initial_intake = if mempool_paused { MempoolIntakeMode::Paused } else { MempoolIntakeMode::Running };
        let (mempool_intake_tx, mempool_intake_rx) = watch::channel(initial_intake);
        Self {
            backend: backend.clone(),
            mempool,
            close_queue_capacity: 1,
            current_state: None,
            metrics,
            handle: BlockProductionHandle::new(
                backend,
                sender,
                bypass_input_sender,
                mempool_intake_tx.clone(),
                no_charge_fee,
            ),
            state_notifications: None,
            executor_commands_recv: Some(recv),
            l1_client,
            bypass_tx_input: Some(bypass_tx_input),
            mempool_intake_rx,
            no_charge_fee,
            replay_mode_enabled: false,
            parallel_merkle_enabled: false,
            parallel_merkle_compare_sequential: false,
            parallel_merkle_root_workers: 1,
            parallel_merkle_flush_interval: 3,
            parallel_merkle_trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode::Checkpoint,
            diffs_since_snapshot: Vec::new(),
            pending_completions: VecDeque::new(),
        }
    }

    pub fn with_close_queue_capacity(mut self, close_queue_capacity: usize) -> Self {
        self.close_queue_capacity = close_queue_capacity.max(1);
        self
    }

    pub fn with_parallel_merkle_enabled(mut self, enabled: bool) -> Self {
        self.parallel_merkle_enabled = enabled;
        self
    }

    pub fn with_parallel_merkle_compare_sequential(mut self, enabled: bool) -> Self {
        self.parallel_merkle_compare_sequential = enabled;
        self
    }

    pub fn with_parallel_merkle_root_workers(mut self, worker_count: u64) -> Self {
        self.parallel_merkle_root_workers = usize::try_from(worker_count).unwrap_or(usize::MAX).max(1);
        self
    }

    pub fn with_replay_mode_enabled(mut self, enabled: bool) -> Self {
        self.replay_mode_enabled = enabled;
        self.handle.set_replay_mode_enabled(enabled);
        self
    }

    pub fn with_parallel_merkle_flush_interval(mut self, flush_interval: u64) -> Self {
        self.parallel_merkle_flush_interval = flush_interval.max(1);
        self
    }

    pub fn with_parallel_merkle_trie_log_mode(
        mut self,
        trie_log_mode: mc_db::rocksdb::global_trie::in_memory::TrieLogMode,
    ) -> Self {
        self.parallel_merkle_trie_log_mode = trie_log_mode;
        self
    }

    pub fn handle(&self) -> BlockProductionHandle {
        self.handle.clone()
    }

    /// This is a channel that helps the testing of the block production task. It is unused outside of tests.
    pub fn subscribe_state_notifications(&mut self) -> mpsc::UnboundedReceiver<BlockProductionStateNotification> {
        let (sender, recv) = mpsc::unbounded_channel();
        self.state_notifications = Some(sender);
        recv
    }

    fn send_state_notification(&mut self, notification: BlockProductionStateNotification) {
        if let Some(sender) = self.state_notifications.as_mut() {
            let _ = sender.send(notification);
        }
    }

    fn record_block_stage_metrics(&self) {
        let executing = u64::from(matches!(self.current_state.as_ref(), Some(TaskState::Executing(_))));
        let pending_close = self.pending_completions.len() as u64;
        let diffs_since_snapshot = self.diffs_since_snapshot.len() as u64;
        let tracked_total = executing.saturating_add(pending_close).saturating_add(diffs_since_snapshot);

        self.metrics.stage_executing_blocks.record(executing, &[]);
        self.metrics.stage_pending_close_completions.record(pending_close, &[]);
        self.metrics.stage_diffs_since_snapshot.record(diffs_since_snapshot, &[]);
        self.metrics.stage_tracked_blocks_total.record(tracked_total, &[]);
    }

    fn close_queue_capacity(&self) -> usize {
        self.close_queue_capacity.max(1)
    }

    fn is_boundary_block(&self, block_n: u64) -> bool {
        let Some(next_block_n) = block_n.checked_add(1) else {
            return false;
        };
        self.parallel_merkle_flush_interval != 0
            && next_block_n.checked_rem(self.parallel_merkle_flush_interval) == Some(0)
    }

    /// Prepares a PreconfirmedExecutedTransaction for re-execution by converting it to blockifier format.
    ///
    /// This function converts a `PreconfirmedExecutedTransaction` (stored in the database) back into a
    /// blockifier transaction format that can be re-executed. It handles all the necessary conversions
    /// and ensures execution flags are properly set.
    ///
    /// # Process
    ///
    /// 1. Converts `PreconfirmedExecutedTransaction` to `ValidatedTransaction` using `to_validated()`
    /// 2. Sets `charge_fee` based on the `no_charge_fee` configuration (`charge_fee = !no_charge_fee`)
    /// 3. Fetches `declared_class` from state if missing (for Declare transactions)
    /// 4. Converts to blockifier format using `into_blockifier_for_sequencing()` which properly applies execution flags
    ///
    /// # Important Notes
    ///
    /// - The `charge_fee` flag is determined by `self.no_charge_fee` configuration. Note that `new()` is
    ///   called every time Madara starts, so there is no guarantee that the `no_charge_fee` value matches
    ///   the value used during original execution. This is a limitation that should be addressed by storing
    ///   execution configuration in the database (see TODO in `new()`).
    /// - For L1 handler transactions, `paid_fee_on_l1` is preserved from `PreconfirmedExecutedTransaction`
    ///   (stored during `append_batch`) and used during conversion via `to_validated()`
    /// - Declare transactions may need their `declared_class` fetched from state if not already stored
    /// - The conversion uses `into_blockifier_for_sequencing()` which properly sets all execution flags
    ///   including `charge_fee`, `validate`, and `only_query`
    fn prepare_preconfirmed_tx_for_reexecution(
        &self,
        preconfirmed_tx: &PreconfirmedExecutedTransaction,
        state_view: &MadaraStateView,
        no_charge_fee: bool,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        // Convert PreconfirmedExecutedTransaction to ValidatedTransaction
        // Use the actual charge_fee value from configuration (charge_fee = !no_charge_fee)
        let mut validated_tx = preconfirmed_tx.to_validated();
        validated_tx.charge_fee = !no_charge_fee;

        // If declared_class is missing and transaction is Declare, fetch it from state_view
        // NOTE: For declare transactions in the preconfirmed block, declared_class MUST be stored
        // during append_batch. If it's None here, that indicates data corruption - we should panic.
        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                // This should never happen for declare transactions in the preconfirmed block
                // If it does, it indicates missing data that should have been stored during original execution
                validated_tx.declared_class = Some(
                    state_view
                        .get_class_info_and_compiled(declare_tx.class_hash())
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "CRITICAL: Error fetching class for class_hash={:#x} in preconfirmed block. \
                                 This indicates data corruption - declared_class should have been stored during append_batch. Error: {}",
                                declare_tx.class_hash(),
                                e
                            )
                        })?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "CRITICAL: Class not found for class_hash={:#x} in parent state view. \
                                 For declare transactions in the preconfirmed block, declared_class must be stored during append_batch.",
                                declare_tx.class_hash()
                            )
                        })?,
                );
            }
        }

        // Use into_blockifier_for_sequencing which properly sets execution flags including charge_fee
        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for reexecution")?;

        Ok(blockifier_tx)
    }

    /// Helper function to close a preconfirmed block with the given state_diff and bouncer weights.
    /// This is used both during normal block closing (EndBlock case) and during restart recovery.
    /// Returns the result including timing information from the DB layer.
    async fn close_preconfirmed_block_with_state_diff(
        backend: Arc<MadaraBackend>,
        block_number: u64,
        consumed_core_contract_nonces: HashSet<u64>,
        bouncer_weights: &blockifier::bouncer::BouncerWeights,
        state_diff: mp_state_update::StateDiff,
    ) -> anyhow::Result<mc_db::AddFullBlockResult> {
        // Copy bouncer_weights to move into the closure (BouncerWeights implements Copy)
        let bouncer_weights = *bouncer_weights;
        global_spawn_rayon_task(move || {
            // Remove consumed L1 to L2 message nonces
            for l1_nonce in consumed_core_contract_nonces {
                backend
                    .remove_pending_message_to_l2(l1_nonce)
                    .context("Removing pending message to l2 from database")?;
            }

            // Save bouncer weights
            backend
                .write_access()
                .write_bouncer_weights(block_number, &bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;

            // Close the preconfirmed block with state_diff
            let result = backend
                .write_access()
                .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, block_number, state_diff)
                .context("Closing preconfirmed block")?;

            anyhow::Ok(result)
        })
        .await
    }

    /// Helper function to get the hash of block_n-10 if it exists.
    fn wait_for_hash_of_block_min_10(
        backend: &Arc<MadaraBackend>,
        block_n: u64,
    ) -> anyhow::Result<Option<(u64, Felt)>> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else {
            return Ok(None);
        };

        if let Some(view) = backend.block_view_on_confirmed(block_n_min_10) {
            let block_hash = view.get_block_info().context("Getting block hash of block_n - 10")?.block_hash;
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            anyhow::bail!(
                "Cannot fetch block #{block_n_min_10} hash (required for block_n-10 context), block view not found"
            )
        }
    }

    /// Re-executes all transactions in a PreconfirmedBlock to obtain BlockExecutionSummary.
    ///
    /// This function is called when Madara restarts with a preconfirmed block in the database.
    /// It recreates the execution context and re-executes all transactions to regenerate:
    /// - `bouncer_weights`: Resource usage metrics required for block finalization
    /// - `state_diff`: Aggregated state changes needed for block closing
    ///
    /// # Process
    ///
    /// 1. Retrieves all executed transactions from the preconfirmed block
    /// 2. Converts them to blockifier format using `prepare_preconfirmed_tx_for_reexecution()`
    /// 3. Creates `BlockExecutionContext` from the preconfirmed block's header (preserving timestamp, gas_prices, etc.)
    /// 4. Sets up `LayeredStateAdapter` for state access
    /// 5. Creates `TransactionExecutor` with proper `block_n-10` state diff handling (Starknet protocol requirement)
    /// 6. Executes all transactions and calls `finalize()` to get `BlockExecutionSummary`
    ///
    /// # Important Notes
    ///
    /// - The execution context uses the exact header values from the preconfirmed block (timestamp, gas_prices, etc.)
    /// - This ensures re-execution produces the same results as the original execution
    /// - The `block_n-10` state diff entry is set on the `0x1` contract address for protocol compliance
    async fn reexecute_preconfirmed_block(
        &self,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        saved_chain_config: Option<&Arc<mp_chain_config::ChainConfig>>,
        saved_no_charge_fee: bool,
    ) -> anyhow::Result<BlockExecutionSummary> {
        // Get all executed transactions
        let executed_txs: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();

        // Get parent block state view
        let parent_state_view = preconfirmed_view.state_view_on_parent();

        // Convert transactions to blockifier format
        // Note: saved_no_charge_fee is passed here to ensure re-execution uses the saved value
        let blockifier_txs: Vec<blockifier::transaction::transaction_execution::Transaction> = executed_txs
            .iter()
            .map(|preconfirmed_tx| {
                self.prepare_preconfirmed_tx_for_reexecution(preconfirmed_tx, &parent_state_view, saved_no_charge_fee)
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Converting preconfirmed transactions to blockifier format")?;

        // Create BlockExecutionContext from PreconfirmedBlock header (preserving exact saved values)
        let header = &preconfirmed_view.block().header;
        let exec_ctx = BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        // Create LayeredStateAdapter
        let state_adapter =
            LayeredStateAdapter::new(self.backend.clone()).context("Creating LayeredStateAdapter for re-execution")?;

        // Create TransactionExecutor with block_n-10 handling
        // Use saved configs if available, otherwise use current backend configs
        let custom_chain_config = saved_chain_config;

        let mut executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state_adapter,
            |block_n| Self::wait_for_hash_of_block_min_10(&self.backend, block_n),
            custom_chain_config, // Use saved chain_config if available (re-execution)
        )
        .context("Creating TransactionExecutor for re-execution")?;

        // Execute all transactions
        let execution_results = executor.execute_txs(&blockifier_txs, /* execution_deadline */ None);

        // Verify that re-execution produces matching receipts
        for (i, (result, preconfirmed_tx)) in execution_results.iter().zip(executed_txs.iter()).enumerate() {
            match result {
                Ok((exec_info, _state_maps)) => {
                    // Convert execution info to receipt
                    let reexecuted_receipt = from_blockifier_execution_info(exec_info, &blockifier_txs[i]);

                    // Compare receipts - they should match exactly
                    anyhow::ensure!(
                        reexecuted_receipt.transaction_hash() == preconfirmed_tx.transaction.receipt.transaction_hash(),
                        "Re-execution produced different receipt hash for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );

                    anyhow::ensure!(
                        reexecuted_receipt == preconfirmed_tx.transaction.receipt,
                        "Re-execution produced different receipt content for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
                Err(err) => {
                    tracing::warn!("Transaction execution error during re-execution: {err:?}");
                    // If execution failed, we can't compare receipts, but this is unexpected
                    anyhow::bail!(
                        "Transaction {} (hash: {:#x}) failed during re-execution: {err:?}",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
            }
        }

        // Call finalize() to get BlockExecutionSummary
        let block_exec_summary = executor.finalize().context("Finalizing executor to get BlockExecutionSummary")?;

        Ok(block_exec_summary)
    }

    /// Saves current runtime config for future restarts.
    fn save_current_runtime_exec_config(&self) -> anyhow::Result<()> {
        let current_chain_config = self.backend.chain_config();
        let current_exec_constants = current_chain_config
            .exec_constants_by_protocol_version(current_chain_config.latest_protocol_version)
            .context("Failed to resolve execution constants for latest protocol version")?;

        let runtime_config = RuntimeExecutionConfig::from_current_config(
            current_chain_config,
            current_exec_constants,
            self.no_charge_fee,
        )
        .context("Failed to create runtime execution config")?;

        self.backend
            .write_access()
            .write_runtime_exec_config(&runtime_config)
            .context("Saving runtime execution config")?;

        Ok(())
    }

    /// Closes the last preconfirmed block stored in the database (if any).
    ///
    /// This function is called when Madara restarts and finds a preconfirmed block in the database.
    /// It handles closing the block properly by re-executing transactions to regenerate execution context.
    ///
    /// # Process
    ///
    /// 1. Checks if a preconfirmed block exists.
    /// 2. Re-executes transactions to obtain `bouncer_weights` and `state_diff`.
    /// 3. Extracts L1 handler nonces and cleans up L1-L2 message nonces.
    /// 4. Saves bouncer weights and closes the block.
    /// 5. Updates runtime config for future blocks.
    ///
    /// Note: Re-execution uses saved config values (e.g. `no_charge_fee`) to ensure consistency with original execution.
    /// Runtime config is always saved for persistence.
    async fn close_preconfirmed_block_if_exists(&mut self) -> anyhow::Result<()> {
        let head = self.backend.chain_head_state();
        let confirmed_tip = head.confirmed_tip;
        let Some(internal_preconfirmed_tip) = head.internal_preconfirmed_tip else {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        };

        // Startup recovery scans block-keyed preconfirmed entries in ascending order:
        // [confirmed + 1, internal_preconfirmed_tip].
        let start_block_n = confirmed_tip.map(|n| n.saturating_add(1)).unwrap_or(0);
        if start_block_n > internal_preconfirmed_tip {
            self.save_current_runtime_exec_config()?;
            return Ok(());
        }

        tracing::debug!(
            "Close preconfirmed blocks on startup from block_n={} to block_n={}",
            start_block_n,
            internal_preconfirmed_tip
        );

        let saved_config = self.backend.get_runtime_exec_config().context("Getting runtime execution config")?;
        let (saved_chain_config, saved_no_charge_fee) = if let Some(config) = saved_config {
            (Some(Arc::new(config.chain_config)), config.no_charge_fee)
        } else {
            tracing::warn!("No saved runtime execution config found, using current configs (backward compatibility)");
            (None, self.no_charge_fee)
        };

        for block_number in start_block_n..=internal_preconfirmed_tip {
            let preconfirmed_view = self
                .backend
                .block_view_on_preconfirmed(block_number)
                .with_context(|| format!("Getting preconfirmed block view for block #{block_number}"))?;

            let n_txs = preconfirmed_view.num_executed_transactions();
            tracing::debug!(
                "Re-executing {} transaction(s) in preconfirmed block #{} to obtain bouncer_weights and state_diff",
                n_txs,
                block_number
            );

            let block_exec_summary = self
                .reexecute_preconfirmed_block(&preconfirmed_view, saved_chain_config.as_ref(), saved_no_charge_fee)
                .await
                .with_context(|| format!("Re-executing preconfirmed block #{block_number} to get execution summary"))?;

            let consumed_core_contract_nonces: HashSet<u64> = preconfirmed_view
                .borrow_content()
                .executed_transactions()
                .filter_map(|tx| tx.transaction.transaction.as_l1_handler().map(|l1_tx| l1_tx.nonce))
                .collect();

            let old_declared_contracts = preconfirmed_view.get_old_declared_contracts();
            let deployed_contracts_set = preconfirmed_view.get_deployed_contracts_set();
            let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
                .compiled_class_hashes_for_migration
                .iter()
                .map(|(v2_hash, _v1_hash)| v2_hash.0)
                .collect();

            let state_diff = mp_state_update::StateDiff::from_blockifier(
                block_exec_summary.state_diff,
                &migration_v2_hashes,
                &deployed_contracts_set,
                old_declared_contracts,
            );

            let _db_result = Self::close_preconfirmed_block_with_state_diff(
                self.backend.clone(),
                block_number,
                consumed_core_contract_nonces,
                &block_exec_summary.bouncer_weights,
                state_diff,
            )
            .await
            .with_context(|| format!("Closing preconfirmed block #{block_number} on startup"))?;

            tracing::info!("✅ Closed preconfirmed block #{} with {} transactions on startup", block_number, n_txs);
        }

        self.save_current_runtime_exec_config()
            .context("Updating runtime execution config after startup preconfirmed recovery")?;

        Ok(())
    }

    /// Handles the state machine and its transitions.
    async fn process_reply(&mut self, reply: ExecutorMessage, close_queue: &FinalizerHandle) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { exec_ctx } => {
                tracing::debug!("received_executor_start_new_block block_n={}", exec_ctx.block_number);
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::NotExecuting { latest_block_n } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let new_block_n = latest_block_n
                    .map(|n| n.checked_add(1).context("Block number overflow while starting new block"))
                    .transpose()?
                    .unwrap_or(/* genesis */ 0);
                if new_block_n != exec_ctx.block_number {
                    anyhow::bail!(
                        "Received new block_n={} from executor, expected block_n={}",
                        exec_ctx.block_number,
                        new_block_n
                    )
                }

                // Check if pre-confirmed block exists (it shouldn't at this point)
                // Create new preconfirmed block
                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header()))
                })
                .await?;

                self.current_state =
                    Some(TaskState::Executing(CurrentBlockState::new(self.backend.clone(), new_block_n)));
                self.record_block_stage_metrics();
            }
            ExecutorMessage::BatchExecuted(batch_execution_result) => {
                let current_state = self.current_state.as_mut().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };
                let batch_exec_secs = batch_execution_result.stats.exec_duration.as_secs_f64();
                let batch_exec_ms = batch_exec_secs * 1000.0;
                let executor_to_main_delivery_secs = batch_execution_result.emitted_at.elapsed().as_secs_f64();
                let executor_to_main_delivery_ms = executor_to_main_delivery_secs * 1000.0;
                self.metrics.executor_batch_execution_duration.record(batch_exec_secs, &[]);
                self.metrics.executor_batch_execution_last.record(batch_exec_secs, &[]);
                self.metrics.executor_to_main_delivery_duration.record(executor_to_main_delivery_secs, &[]);
                self.metrics.executor_to_main_delivery_last.record(executor_to_main_delivery_secs, &[]);
                tracing::debug!(
                    "received_executor_batch_executed block_number={} txs_executed_in_batch={} txs_added_to_block={} txs_reverted={} txs_rejected={} batch_exec_duration_ms={} executor_to_main_delivery_ms={} close_queue_depth={} close_queue_in_flight={} pending_close_completions={}",
                    state.block_number,
                    batch_execution_result.stats.n_executed,
                    batch_execution_result.stats.n_added_to_block,
                    batch_execution_result.stats.n_reverted,
                    batch_execution_result.stats.n_rejected,
                    batch_exec_ms,
                    executor_to_main_delivery_ms,
                    close_queue.current_depth(),
                    close_queue.current_in_flight(),
                    self.pending_completions.len()
                );

                // Record batch execution stats metrics
                self.metrics.record_execution_stats(&batch_execution_result.stats);

                // Accumulate stats for the log event at block close
                state.accumulated_stats = state.accumulated_stats.clone() + batch_execution_result.stats.clone();
                state.last_execution_finished_at = Some(batch_execution_result.emitted_at);

                state.append_batch(batch_execution_result).await?;

                self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
            }
            ExecutorMessage::EndBlock(block_exec_summary) => {
                tracing::debug!("received_executor_end_block");
                self.close_block(block_exec_summary, close_queue).await?;
            }
            ExecutorMessage::EndFinalBlock(block_exec_summary) => {
                tracing::debug!("received_executor_end_final_block");
                match block_exec_summary {
                    Some(summary) => {
                        self.close_block(summary, close_queue).await?;
                    }
                    None => {
                        tracing::debug!("EndFinalBlock(None) received - executor completed without block");
                    }
                }
            }
        }

        Ok(())
    }

    /// Close and save a block using the execution summary.
    /// Used for both normal block closing (EndBlock) and shutdown (EndFinalBlock).
    async fn close_block(
        &mut self,
        block_exec_summary: Box<BlockExecutionSummary>,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let current_state = self.current_state.take().context("No current state")?;
        let TaskState::Executing(state) = current_state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
        };
        let block_n = state.block_number;
        let close_block_received_at = Instant::now();
        let last_execution_finished_at = state.last_execution_finished_at;
        let executor_to_close_queue =
            last_execution_finished_at.map(|finished_at| close_block_received_at.duration_since(finished_at));
        tracing::debug!(
            "close_block_received_from_executor block_number={} executor_to_close_queue_ms={:?}",
            block_n,
            executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0)
        );

        let preconfirmed_view = state
            .backend
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("No pre-confirmed block #{block_n}"))?;
        let old_declared_contracts = preconfirmed_view.get_old_declared_contracts();
        let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();
        let state_diff = mp_state_update::StateDiff::from_blockifier(
            block_exec_summary.state_diff.clone(),
            &migration_v2_hashes,
            &state.deployed_contracts,
            old_declared_contracts,
        );

        let is_boundary = self.is_boundary_block(block_n);
        let (root_base_block_n, root_snapshot, root_state_diffs) = if self.parallel_merkle_enabled {
            self.diffs_since_snapshot.push((block_n, state_diff.clone()));
            let generic_floor = self.backend.db.get_latest_snapshot_floor(block_n.checked_sub(1));
            let (base_block_n, snapshot) =
                self.backend.db.get_latest_durable_snapshot_floor(block_n.checked_sub(1)).ok_or_else(|| {
                    anyhow::anyhow!("Missing durable snapshot floor for root computation of block #{block_n}")
                })?;
            if let Some((generic_block_n, _)) = generic_floor {
                if generic_block_n != base_block_n {
                    tracing::debug!(
                        "parallel_root_non_durable_floor_ignored block_number={} generic_base_snapshot_block={generic_block_n:?} durable_base_snapshot_block={base_block_n:?}",
                        block_n
                    );
                }
            }
            let root_state_diffs = collect_diffs_for_root_from_base(&self.diffs_since_snapshot, base_block_n, block_n)?;
            tracing::debug!(
                "parallel_root_job_enqueued block_number={} base_snapshot_block={base_block_n:?} diff_count={} squashed_block_count={} diff_start_block={} diff_end_block={} include_overlay={} trie_log_mode={:?} durable_base=true active_parallel_root_jobs={}",
                block_n,
                root_state_diffs.len(),
                root_state_diffs.len(),
                base_block_n.map_or(0, |base| base.saturating_add(1)),
                block_n,
                is_boundary,
                self.parallel_merkle_trie_log_mode,
                active_parallel_root_jobs()
            );
            (base_block_n, Some(snapshot), root_state_diffs)
        } else {
            (None, None, Vec::new())
        };

        let enqueued_at = Instant::now();
        if let Some(duration) = executor_to_close_queue {
            self.metrics.executor_to_close_queue_duration.record(duration.as_secs_f64(), &[]);
            self.metrics.executor_to_close_queue_last.record(duration.as_secs_f64(), &[]);
        }
        let close_block_enqueue_duration = enqueued_at.duration_since(close_block_received_at);
        self.metrics.close_block_enqueue_duration.record(close_block_enqueue_duration.as_secs_f64(), &[]);
        self.metrics.close_block_enqueue_last.record(close_block_enqueue_duration.as_secs_f64(), &[]);
        let payload = QueuedClosePayload {
            close_job_payload: mc_db::close_pipeline_contract::CloseJobPayload { block_n },
            state,
            block_exec_summary,
            state_diff,
            is_boundary,
            trie_log_mode: self.parallel_merkle_trie_log_mode,
            compare_parallel_with_sequential: self.parallel_merkle_compare_sequential,
            root_base_block_n,
            root_snapshot,
            root_state_diffs,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
        };
        let (queued_result, completion) = close_queue.try_enqueue(payload)?;
        let ClosePreconfirmedResult::Queued(queued_meta) = queued_result;
        let queue_depth = close_queue.current_depth();
        let queue_in_flight = close_queue.current_in_flight();
        let pending_close_completions = if self.parallel_merkle_enabled {
            self.pending_completions.len() + 1
        } else {
            self.pending_completions.len()
        };
        self.metrics.close_queue_enqueued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(queue_depth as u64, &[]);
        tracing::debug!(
            "close_block_queued block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle={} executor_to_close_queue_ms={:?} close_block_to_queue_enqueue_ms={}",
            queued_meta.block_n,
            queue_depth,
            close_queue.configured_capacity(),
            queue_in_flight,
            pending_close_completions,
            self.parallel_merkle_enabled,
            executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0),
            close_block_enqueue_duration.as_secs_f64() * 1000.0
        );

        if self.parallel_merkle_enabled {
            self.pending_completions.push_back((block_n, completion));
            self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(block_n) });
            self.record_block_stage_metrics();
            tracing::debug!(
                "parallel_merkle_close_deferred block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} root_compute_background=true",
                block_n,
                close_queue.current_depth(),
                close_queue.configured_capacity(),
                close_queue.current_in_flight(),
                self.pending_completions.len()
            );
            return Ok(());
        }

        let completion = completion.await.context("Close queue worker dropped completion channel")??;
        self.metrics.close_queue_dequeued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(close_queue.current_depth() as u64, &[]);
        tracing::debug!(
            "close_block_complete block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle={}",
            completion.block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            self.pending_completions.len(),
            self.parallel_merkle_enabled
        );
        if let Some(status) = self.backend.replay_boundary_mark_closed(completion.block_n) {
            if !status.boundary_met {
                tracing::warn!(
                    "replay_boundary_closed_without_match block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} mismatch={:?}",
                    status.block_n,
                    status.expected_tx_count,
                    status.executed_tx_count,
                    status.dispatched_tx_count,
                    status.reached_last_tx_hash,
                    status.mismatch
                );
            }
        }

        self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(completion.block_n) });
        self.send_state_notification(BlockProductionStateNotification::ClosedBlock { block_n: completion.block_n });
        self.record_block_stage_metrics();

        Ok(())
    }

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
        let start_time = Instant::now();
        let executor_to_close_queue =
            last_execution_finished_at.map(|finished_at| close_block_received_at.duration_since(finished_at));
        let close_block_to_queue_enqueue = enqueued_at.duration_since(close_block_received_at);

        // Get preconfirmed block view for transaction count and old_declared_contracts
        let preconfirmed_view = state
            .backend
            .block_view_on_preconfirmed(state.block_number)
            .with_context(|| format!("No pre-confirmed block #{}", state.block_number))?;
        let n_txs = preconfirmed_view.num_executed_transactions();
        let event_count = preconfirmed_view
            .borrow_content()
            .executed_transactions()
            .map(|tx| tx.transaction.receipt.events().len() as u64)
            .sum::<u64>();
        let declared_classes_count = state_diff.declared_classes.len();
        let deployed_contracts_count = state_diff.deployed_contracts.len();
        let storage_diffs_count = state_diff.storage_diffs.len();
        let nonce_updates_count = state_diff.nonces.len();
        let state_diff_len = state_diff.len();
        let consumed_l1_nonces_count = state.consumed_core_contract_nonces.len();

        let bouncer_l1_gas = block_exec_summary.bouncer_weights.l1_gas;
        let bouncer_sierra_gas = block_exec_summary.bouncer_weights.sierra_gas.0;
        let bouncer_n_events = block_exec_summary.bouncer_weights.n_events;
        let bouncer_message_segment_length = block_exec_summary.bouncer_weights.message_segment_length;
        let bouncer_state_diff_size = block_exec_summary.bouncer_weights.state_diff_size;

        metrics.block_declared_classes_count.record(declared_classes_count as u64, &[]);
        metrics.block_deployed_contracts_count.record(deployed_contracts_count as u64, &[]);
        metrics.block_storage_diffs_count.record(storage_diffs_count as u64, &[]);
        metrics.block_nonce_updates_count.record(nonce_updates_count as u64, &[]);
        metrics.block_state_diff_length.record(state_diff_len as u64, &[]);
        metrics.block_event_count.record(event_count, &[]);

        metrics.block_bouncer_l1_gas.record(bouncer_l1_gas as u64, &[]);
        metrics.block_bouncer_sierra_gas.record(bouncer_sierra_gas, &[]);
        metrics.block_bouncer_n_events.record(bouncer_n_events as u64, &[]);
        metrics.block_bouncer_message_segment_length.record(bouncer_message_segment_length as u64, &[]);
        metrics.block_bouncer_state_diff_size.record(bouncer_state_diff_size as u64, &[]);
        metrics.block_consumed_l1_nonces_count.record(consumed_l1_nonces_count as u64, &[]);

        let close_preconfirmed_start = Instant::now();
        let db_result = Self::close_preconfirmed_block_with_state_diff(
            state.backend.clone(),
            state.block_number,
            state.consumed_core_contract_nonces,
            &block_exec_summary.bouncer_weights,
            state_diff,
        )
        .await
        .context("Closing block")?;
        let close_preconfirmed_duration = close_preconfirmed_start.elapsed();
        metrics.close_preconfirmed_duration.record(close_preconfirmed_duration.as_secs_f64(), &[]);
        metrics.close_preconfirmed_last.record(close_preconfirmed_duration.as_secs_f64(), &[]);

        let close_commit_stage_duration = start_time.elapsed();
        let close_end_to_end_duration = close_block_received_at.elapsed();
        let block_production_time = state.block_start_time.elapsed();
        let close_post_execution_duration = last_execution_finished_at
            .map(|finished_at| close_block_received_at.duration_since(finished_at) + close_end_to_end_duration);

        let timings = &db_result.timings;
        let exec_stats = &state.accumulated_stats;
        tracing::info!(
            target: "close_block",
            block_number = state.block_number,
            tx_count = n_txs,
            event_count = event_count,
            close_block_total_ms = close_end_to_end_duration.as_secs_f64() * 1000.0,
            close_end_to_end_ms = close_end_to_end_duration.as_secs_f64() * 1000.0,
            close_commit_stage_ms = close_commit_stage_duration.as_secs_f64() * 1000.0,
            close_post_execution_ms = ?close_post_execution_duration.map(|duration| duration.as_secs_f64() * 1000.0),
            block_close_ms = close_commit_stage_duration.as_secs_f64() * 1000.0,
            close_preconfirmed_ms = close_preconfirmed_duration.as_secs_f64() * 1000.0,
            block_production_ms = block_production_time.as_secs_f64() * 1000.0,
            block_lifetime_ms = block_production_time.as_secs_f64() * 1000.0,
            execution_total_ms = exec_stats.exec_duration.as_secs_f64() * 1000.0,
            executor_to_close_queue_ms = ?executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0),
            close_block_to_queue_enqueue_ms = close_block_to_queue_enqueue.as_secs_f64() * 1000.0,
            batches_executed = exec_stats.n_batches,
            txs_added_to_block = exec_stats.n_added_to_block,
            txs_executed = exec_stats.n_executed,
            txs_reverted = exec_stats.n_reverted,
            txs_rejected = exec_stats.n_rejected,
            classes_declared = exec_stats.declared_classes,
            l2_gas_consumed = exec_stats.l2_gas_consumed,
            state_diff_len = state_diff_len,
            declared_classes = declared_classes_count,
            deployed_contracts = deployed_contracts_count,
            storage_diffs = storage_diffs_count,
            nonce_updates = nonce_updates_count,
            consumed_l1_nonces = consumed_l1_nonces_count,
            bouncer_l1_gas = bouncer_l1_gas,
            bouncer_sierra_gas = bouncer_sierra_gas,
            bouncer_n_events = bouncer_n_events,
            bouncer_message_segment_length = bouncer_message_segment_length,
            bouncer_state_diff_size = bouncer_state_diff_size,
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
        record_closed_block_summary_metrics(
            &metrics,
            state.block_number,
            n_txs as u64,
            exec_stats.exec_duration,
            close_commit_stage_duration,
            close_end_to_end_duration,
            close_post_execution_duration,
            block_production_time,
            exec_stats,
        );

        Ok(CloseJobCompletion { block_n: state.block_number })
    }

    async fn execute_close_payload_batch(
        metrics: Arc<BlockProductionMetrics>,
        payloads: Vec<QueuedClosePayload>,
    ) -> Vec<anyhow::Result<CloseJobCompletion>> {
        let mut results = Vec::with_capacity(payloads.len());
        for payload in payloads {
            results.push(Self::execute_close_payload(metrics.clone(), payload).await);
        }
        results
    }

    /// Parallel merkle stage 1: precompute the root for a single block.
    ///
    /// This stage is safe to run truly in parallel across blocks because it does
    /// not mutate the confirmed head or the persisted block stream.
    async fn compute_close_payload_parallel_root(
        metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
    ) -> anyhow::Result<ParallelComputedClosePayload> {
        let QueuedClosePayload {
            close_job_payload,
            state,
            block_exec_summary,
            state_diff,
            is_boundary,
            trie_log_mode,
            compare_parallel_with_sequential,
            root_base_block_n,
            root_snapshot,
            root_state_diffs,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
        } = payload;
        let block_n = state.block_number;
        let squashed_block_count = root_state_diffs.len();
        let diff_start_block = root_base_block_n.map(|base| base.saturating_add(1)).or(Some(block_n));
        let active_parallel_root_jobs_on_dispatch = active_parallel_root_jobs();
        tracing::debug!(
            "parallel_root_single_block_dispatch block_number={} base_snapshot_block={root_base_block_n:?} diff_count={} squashed_block_count={} diff_start_block={diff_start_block:?} diff_end_block={} include_overlay={} trie_log_mode={:?} active_parallel_root_jobs={}",
            block_n,
            squashed_block_count,
            squashed_block_count,
            block_n,
            is_boundary,
            trie_log_mode,
            active_parallel_root_jobs_on_dispatch
        );
        let root_wait_started_at = Instant::now();
        let backend = Arc::clone(&state.backend);
        let metrics_for_compute = Arc::clone(&metrics);
        let snapshot = root_snapshot.context("Missing root snapshot for parallel close job")?;
        let state_diffs_for_compute = root_state_diffs.clone();
        let dispatched_at = Instant::now();
        let (root_response, active_parallel_root_jobs_on_start, active_parallel_root_jobs_before_finish, active_parallel_root_jobs_after_finish, squash_state_diffs_duration, root_spawn_blocking_queue_duration, root_compute_duration, root_total_duration) = tokio::task::spawn_blocking(move || {
            let closure_started_at = Instant::now();
            let spawn_blocking_queue_duration = closure_started_at.duration_since(dispatched_at);
            metrics_for_compute
                .parallel_root_spawn_blocking_queue_duration
                .record(spawn_blocking_queue_duration.as_secs_f64(), &[]);
            metrics_for_compute
                .parallel_root_spawn_blocking_queue_last
                .record(spawn_blocking_queue_duration.as_secs_f64(), &[]);
            let (root_job_guard, active_parallel_root_jobs_on_start) = ParallelRootJobGuard::acquire();
            tracing::debug!(
                "parallel_root_single_block_compute_started block_number={} base_snapshot_block={root_base_block_n:?} diff_count={} squashed_block_count={} include_overlay={} trie_log_mode={:?} spawn_blocking_queue_ms={} active_parallel_root_jobs={}",
                block_n,
                state_diffs_for_compute.len(),
                state_diffs_for_compute.len(),
                is_boundary,
                trie_log_mode,
                spawn_blocking_queue_duration.as_secs_f64() * 1000.0,
                active_parallel_root_jobs_on_start
            );

            let squash_state_diffs_started_at = Instant::now();
            let cumulative_state_diff =
                mc_db::rocksdb::global_trie::in_memory::squash_state_diffs(state_diffs_for_compute.iter());
            let squash_state_diffs_duration = squash_state_diffs_started_at.elapsed();
            let root_compute_started_at = Instant::now();
            let result = backend.db.compute_root_from_selected_snapshot(
                root_base_block_n,
                snapshot,
                block_n,
                &cumulative_state_diff,
                is_boundary,
                trie_log_mode,
                compare_parallel_with_sequential,
            );
            let compute_duration = root_compute_started_at.elapsed();
            let total_duration = dispatched_at.elapsed();
            metrics_for_compute.parallel_root_compute_duration.record(compute_duration.as_secs_f64(), &[]);
            metrics_for_compute.parallel_root_compute_last.record(compute_duration.as_secs_f64(), &[]);
            metrics_for_compute.parallel_root_total_duration.record(total_duration.as_secs_f64(), &[]);
            metrics_for_compute.parallel_root_total_last.record(total_duration.as_secs_f64(), &[]);
            let active_parallel_root_jobs_before_finish = active_parallel_root_jobs();
            tracing::debug!(
                "parallel_root_single_block_compute_finished block_number={} base_snapshot_block={root_base_block_n:?} diff_count={} squashed_block_count={} include_overlay={} trie_log_mode={:?} success={} squash_state_diffs_ms={} compute_ms={} total_ms={} active_parallel_root_jobs={}",
                block_n,
                state_diffs_for_compute.len(),
                state_diffs_for_compute.len(),
                is_boundary,
                trie_log_mode,
                result.is_ok(),
                squash_state_diffs_duration.as_secs_f64() * 1000.0,
                compute_duration.as_secs_f64() * 1000.0,
                total_duration.as_secs_f64() * 1000.0,
                active_parallel_root_jobs_before_finish
            );
            drop(root_job_guard);
            let active_parallel_root_jobs_after_finish = active_parallel_root_jobs();
            result.map(|root_response| {
                (
                    root_response,
                    active_parallel_root_jobs_on_start,
                    active_parallel_root_jobs_before_finish,
                    active_parallel_root_jobs_after_finish,
                    squash_state_diffs_duration,
                    spawn_blocking_queue_duration,
                    compute_duration,
                    total_duration,
                )
            })
        })
        .await
        .map_err(|error| {
            metrics.parallel_root_failures_total.add(1, &[]);
            anyhow::anyhow!("Parallel merkle blocking task panicked for block #{block_n}: {error:#}")
        })?
        .map_err(|error| {
            metrics.parallel_root_failures_total.add(1, &[]);
            error
        })
        .context(format!("Parallel merkle root computation for block #{block_n}"))?;

        let root_wait_duration = root_wait_started_at.elapsed();
        metrics.parallel_root_await_duration.record(root_wait_duration.as_secs_f64(), &[]);
        metrics.parallel_root_await_last.record(root_wait_duration.as_secs_f64(), &[]);
        tracing::debug!(
            "parallel_root_await_finished block_number={} root_wait_ms={} real_parallel_merkle=true",
            block_n,
            root_wait_duration.as_secs_f64() * 1000.0
        );
        let has_boundary_overlay = root_response.overlay.is_some();

        Ok(ParallelComputedClosePayload {
            payload: QueuedClosePayload {
                close_job_payload,
                state,
                block_exec_summary,
                state_diff,
                is_boundary,
                trie_log_mode,
                compare_parallel_with_sequential,
                root_base_block_n,
                root_snapshot: None,
                root_state_diffs: Vec::new(),
                last_execution_finished_at,
                close_block_received_at,
                enqueued_at,
            },
            root_response,
            parallel_summary: ParallelMerkleSummary {
                base_snapshot_block: root_base_block_n,
                squashed_block_count,
                diff_start_block,
                diff_end_block: block_n,
                active_parallel_root_jobs_on_dispatch,
                active_parallel_root_jobs_on_start,
                active_parallel_root_jobs_before_finish,
                active_parallel_root_jobs_after_finish,
                root_spawn_blocking_queue: root_spawn_blocking_queue_duration,
                root_wait: root_wait_duration,
                squash_state_diffs: squash_state_diffs_duration,
                root_compute: root_compute_duration,
                root_total: root_total_duration,
                boundary_flush: None,
                has_boundary_overlay,
            },
        })
    }

    async fn execute_close_payload_parallel_precomputed_job(
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

    async fn execute_close_payload_parallel_precomputed(
        metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
        root_response: mc_db::rocksdb::global_trie::in_memory::InMemoryRootComputation,
        mut parallel_summary: ParallelMerkleSummary,
    ) -> anyhow::Result<CloseJobCompletion> {
        let QueuedClosePayload {
            state,
            block_exec_summary,
            state_diff,
            trie_log_mode,
            last_execution_finished_at,
            close_block_received_at,
            enqueued_at,
            ..
        } = payload;
        tracing::debug!("close_block_worker_started block_number={} parallel_merkle=true", state.block_number);
        let start_time = Instant::now();
        let executor_to_close_queue =
            last_execution_finished_at.map(|finished_at| close_block_received_at.duration_since(finished_at));
        let close_block_to_queue_enqueue = enqueued_at.duration_since(close_block_received_at);

        // Get preconfirmed block view for transaction count and old_declared_contracts
        let preconfirmed_view = state
            .backend
            .block_view_on_preconfirmed(state.block_number)
            .with_context(|| format!("No pre-confirmed block #{}", state.block_number))?;
        let n_txs = preconfirmed_view.num_executed_transactions();
        let event_count = preconfirmed_view
            .borrow_content()
            .executed_transactions()
            .map(|tx| tx.transaction.receipt.events().len() as u64)
            .sum::<u64>();
        let declared_classes_count = state_diff.declared_classes.len();
        let deployed_contracts_count = state_diff.deployed_contracts.len();
        let storage_diffs_count = state_diff.storage_diffs.len();
        let nonce_updates_count = state_diff.nonces.len();
        let state_diff_len = state_diff.len();
        let consumed_l1_nonces_count = state.consumed_core_contract_nonces.len();

        let bouncer_l1_gas = block_exec_summary.bouncer_weights.l1_gas;
        let bouncer_sierra_gas = block_exec_summary.bouncer_weights.sierra_gas.0;
        let bouncer_n_events = block_exec_summary.bouncer_weights.n_events;
        let bouncer_message_segment_length = block_exec_summary.bouncer_weights.message_segment_length;
        let bouncer_state_diff_size = block_exec_summary.bouncer_weights.state_diff_size;

        metrics.block_declared_classes_count.record(declared_classes_count as u64, &[]);
        metrics.block_deployed_contracts_count.record(deployed_contracts_count as u64, &[]);
        metrics.block_storage_diffs_count.record(storage_diffs_count as u64, &[]);
        metrics.block_nonce_updates_count.record(nonce_updates_count as u64, &[]);
        metrics.block_state_diff_length.record(state_diff_len as u64, &[]);
        metrics.block_event_count.record(event_count, &[]);

        metrics.block_bouncer_l1_gas.record(bouncer_l1_gas as u64, &[]);
        metrics.block_bouncer_sierra_gas.record(bouncer_sierra_gas, &[]);
        metrics.block_bouncer_n_events.record(bouncer_n_events as u64, &[]);
        metrics.block_bouncer_message_segment_length.record(bouncer_message_segment_length as u64, &[]);
        metrics.block_bouncer_state_diff_size.record(bouncer_state_diff_size as u64, &[]);
        metrics.block_consumed_l1_nonces_count.record(consumed_l1_nonces_count as u64, &[]);

        let close_preconfirmed_start = Instant::now();

        // Copy bouncer_weights to move into the closure
        let bouncer_weights = block_exec_summary.bouncer_weights;
        let block_number = state.block_number;
        let consumed_nonces = state.consumed_core_contract_nonces;
        let backend = state.backend.clone();
        let state_diff_len_for_close_pipeline = state_diff_len;
        let has_boundary_overlay = root_response.overlay.is_some();
        tracing::debug!(
            "parallel_close_db_pipeline_started block_number={} state_diff_len={} has_boundary_overlay={}",
            block_number,
            state_diff_len_for_close_pipeline,
            has_boundary_overlay
        );

        let db_result = global_spawn_rayon_task(move || {
            let pipeline_start = Instant::now();
            // Remove consumed L1 to L2 message nonces
            let consumed_nonces_count = consumed_nonces.len();
            let nonce_cleanup_start = Instant::now();
            for l1_nonce in consumed_nonces {
                backend
                    .remove_pending_message_to_l2(l1_nonce)
                    .context("Removing pending message to l2 from database")?;
            }
            tracing::debug!(
                "parallel_close_phase_nonce_cleanup_done block_number={} consumed_nonces={} duration_ms={}",
                block_number,
                consumed_nonces_count,
                nonce_cleanup_start.elapsed().as_secs_f64() * 1000.0
            );

            // Save bouncer weights
            let write_bouncer_start = Instant::now();
            backend
                .write_access()
                .write_bouncer_weights(block_number, &bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;
            tracing::debug!(
                "parallel_close_phase_bouncer_write_done block_number={} duration_ms={}",
                block_number,
                write_bouncer_start.elapsed().as_secs_f64() * 1000.0
            );

            let boundary_storage_contracts: Vec<_> = state_diff
                .storage_diffs
                .iter()
                .map(|item| (item.address, item.storage_entries.len()))
                .collect();

            // Phase 1: write block parts with precomputed root (no head advance yet).
            let write_parts_start = Instant::now();
            let result = backend
                .write_access()
                .write_preconfirmed_with_precomputed_root(
                    /* pre_v0_13_2_hash_override */ true,
                    block_number,
                    state_diff,
                    root_response.state_root,
                    root_response.timings,
                )
                .context("Closing preconfirmed block with precomputed root")?;
            tracing::debug!(
                "parallel_close_phase_write_parts_done block_number={} duration_ms={} merklization_ms={} commitments_ms={} block_hash_ms={} db_write_ms={}",
                block_number,
                write_parts_start.elapsed().as_secs_f64() * 1000.0,
                result.timings.merklization.as_secs_f64() * 1000.0,
                result.timings.block_commitments_compute.as_secs_f64() * 1000.0,
                result.timings.block_hash_compute.as_secs_f64() * 1000.0,
                result.timings.db_write_block_parts.as_secs_f64() * 1000.0
            );

            // Phase 2: boundary durability (if any).
            let mut boundary_flush_duration = None;
            if let Some(overlay) = root_response.overlay.as_ref() {
                let boundary_flush_start = Instant::now();
                backend
                    .db
                    .flush_overlay_and_checkpoint(block_number, overlay, trie_log_mode)
                    .context("Flushing boundary overlay and writing parallel-merkle checkpoint")?;
                let boundary_flush_elapsed = boundary_flush_start.elapsed();
                boundary_flush_duration = Some(boundary_flush_elapsed);
                tracing::debug!(
                    "parallel_close_phase_boundary_flush_done block_number={} duration_ms={} contract_changes={} contract_storage_changes={} class_changes={}",
                    block_number,
                    boundary_flush_elapsed.as_secs_f64() * 1000.0,
                    overlay.contract_changed.len(),
                    overlay.contract_storage_changed.len(),
                    overlay.class_changed.len()
                );
                tracing::debug!(
                    "parallel_boundary_checkpoint_written block_number={} duration_ms={} latest_checkpoint={:?} checkpoint_floor_for_block={:?} trie_log_mode={:?}",
                    block_number,
                    boundary_flush_elapsed.as_secs_f64() * 1000.0,
                    backend.db.get_parallel_merkle_latest_checkpoint().ok().flatten(),
                    backend.db.get_parallel_merkle_checkpoint_floor(block_number).ok().flatten(),
                    trie_log_mode
                );
                let persisted_storage_roots_start = Instant::now();
                let persisted_contract_storage_trie = backend.db.contract_storage_trie();
                for (contract_address, storage_entries_len) in &boundary_storage_contracts {
                    let persisted_storage_root = persisted_contract_storage_trie
                        .root_hash(&contract_address.to_bytes_be())
                        .map_err(mc_db::rocksdb::trie::WrappedBonsaiError)
                        .context("Reading persisted contract storage root after boundary flush")?;
                    tracing::debug!(
                        "parallel_boundary_persisted_storage_root block_number={} contract_address={:#x} storage_entries={} persisted_storage_root={:#x}",
                        block_number,
                        contract_address,
                        storage_entries_len,
                        persisted_storage_root
                    );
                }
                tracing::debug!(
                    "parallel_boundary_persisted_storage_roots_done block_number={} touched_contracts={} duration_ms={}",
                    block_number,
                    boundary_storage_contracts.len(),
                    persisted_storage_roots_start.elapsed().as_secs_f64() * 1000.0
                );
            }

            // Phase 3: only after successful write(+flush), advance confirmed head + GC side effects.
            let confirm_phase_start = Instant::now();
            backend
                .write_access()
                .new_confirmed_block(block_number)
                .context("Advancing confirmed head after parallel close write/flush")?;
            let head_after_confirm = backend.chain_head_state();
            tracing::debug!(
                "parallel_close_phase_confirm_done block_number={} duration_ms={} confirmed_tip={:?} external_preconfirmed_tip={:?} internal_preconfirmed_tip={:?}",
                block_number,
                confirm_phase_start.elapsed().as_secs_f64() * 1000.0,
                head_after_confirm.confirmed_tip,
                head_after_confirm.external_preconfirmed_tip,
                head_after_confirm.internal_preconfirmed_tip
            );
            tracing::debug!(
                "parallel_close_db_pipeline_finished block_number={} total_duration_ms={}",
                block_number,
                pipeline_start.elapsed().as_secs_f64() * 1000.0
            );

            anyhow::Ok((result, boundary_flush_duration))
        })
        .await?;

        let close_preconfirmed_duration = close_preconfirmed_start.elapsed();
        metrics.close_preconfirmed_duration.record(close_preconfirmed_duration.as_secs_f64(), &[]);
        metrics.close_preconfirmed_last.record(close_preconfirmed_duration.as_secs_f64(), &[]);

        let close_commit_stage_duration = start_time.elapsed();
        let close_end_to_end_duration = close_block_received_at.elapsed();
        let block_production_time = state.block_start_time.elapsed();
        let close_post_execution_duration = last_execution_finished_at
            .map(|finished_at| close_block_received_at.duration_since(finished_at) + close_end_to_end_duration);

        let (db_result, boundary_flush_duration) = db_result;
        parallel_summary.boundary_flush = boundary_flush_duration;
        parallel_summary.has_boundary_overlay = has_boundary_overlay;
        let timings = &db_result.timings;
        let exec_stats = &state.accumulated_stats;
        tracing::info!(
            target: "close_block",
            block_number = state.block_number,
            tx_count = n_txs,
            event_count = event_count,
            close_block_total_ms = close_end_to_end_duration.as_secs_f64() * 1000.0,
            close_end_to_end_ms = close_end_to_end_duration.as_secs_f64() * 1000.0,
            close_commit_stage_ms = close_commit_stage_duration.as_secs_f64() * 1000.0,
            close_post_execution_ms = ?close_post_execution_duration.map(|duration| duration.as_secs_f64() * 1000.0),
            block_close_ms = close_commit_stage_duration.as_secs_f64() * 1000.0,
            close_preconfirmed_ms = close_preconfirmed_duration.as_secs_f64() * 1000.0,
            block_production_ms = block_production_time.as_secs_f64() * 1000.0,
            block_lifetime_ms = block_production_time.as_secs_f64() * 1000.0,
            execution_total_ms = exec_stats.exec_duration.as_secs_f64() * 1000.0,
            executor_to_close_queue_ms = ?executor_to_close_queue.map(|duration| duration.as_secs_f64() * 1000.0),
            close_block_to_queue_enqueue_ms = close_block_to_queue_enqueue.as_secs_f64() * 1000.0,
            batches_executed = exec_stats.n_batches,
            txs_added_to_block = exec_stats.n_added_to_block,
            txs_executed = exec_stats.n_executed,
            txs_reverted = exec_stats.n_reverted,
            txs_rejected = exec_stats.n_rejected,
            classes_declared = exec_stats.declared_classes,
            l2_gas_consumed = exec_stats.l2_gas_consumed,
            state_diff_len = state_diff_len,
            declared_classes = declared_classes_count,
            deployed_contracts = deployed_contracts_count,
            storage_diffs = storage_diffs_count,
            nonce_updates = nonce_updates_count,
            consumed_l1_nonces = consumed_l1_nonces_count,
            bouncer_l1_gas = bouncer_l1_gas,
            bouncer_sierra_gas = bouncer_sierra_gas,
            bouncer_n_events = bouncer_n_events,
            bouncer_message_segment_length = bouncer_message_segment_length,
            bouncer_state_diff_size = bouncer_state_diff_size,
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
            base_snapshot_block = ?parallel_summary.base_snapshot_block,
            squashed_block_count = parallel_summary.squashed_block_count,
            diff_start_block = ?parallel_summary.diff_start_block,
            diff_end_block = parallel_summary.diff_end_block,
            active_parallel_root_jobs_on_dispatch = parallel_summary.active_parallel_root_jobs_on_dispatch,
            active_parallel_root_jobs_on_start = parallel_summary.active_parallel_root_jobs_on_start,
            active_parallel_root_jobs_before_finish = parallel_summary.active_parallel_root_jobs_before_finish,
            active_parallel_root_jobs_after_finish = parallel_summary.active_parallel_root_jobs_after_finish,
            root_spawn_blocking_queue_ms = parallel_summary.root_spawn_blocking_queue.as_secs_f64() * 1000.0,
            root_wait_ms = parallel_summary.root_wait.as_secs_f64() * 1000.0,
            squash_state_diffs_ms = parallel_summary.squash_state_diffs.as_secs_f64() * 1000.0,
            root_compute_ms = parallel_summary.root_compute.as_secs_f64() * 1000.0,
            root_total_ms = parallel_summary.root_total.as_secs_f64() * 1000.0,
            boundary_flush_ms = ?parallel_summary.boundary_flush.map(|duration| duration.as_secs_f64() * 1000.0),
            has_boundary_overlay = parallel_summary.has_boundary_overlay,
            parallel_merkle = true,
            "close_block_complete"
        );
        record_closed_block_summary_metrics(
            &metrics,
            state.block_number,
            n_txs as u64,
            exec_stats.exec_duration,
            close_commit_stage_duration,
            close_end_to_end_duration,
            close_post_execution_duration,
            block_production_time,
            exec_stats,
        );

        Ok(CloseJobCompletion { block_n: state.block_number })
    }

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_preconfirmed_block_if_exists().await.context("Cannot close preconfirmed block on startup")?;

        // initial state
        let latest_block_n = self.backend.latest_confirmed_block_n();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n });
        self.record_block_stage_metrics();

        Ok(())
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.setup_initial_state().await?;
        self.metrics.close_queue_depth.record(0, &[]);
        self.metrics.close_queue_in_flight.record(0, &[]);
        self.record_block_stage_metrics();

        let mut executor = executor::start_executor_thread(
            Arc::clone(&self.backend),
            self.executor_commands_recv.take().context("Task already started")?,
            self.metrics.clone(),
            self.replay_mode_enabled,
        )
        .context("Starting executor thread")?;

        let close_queue_capacity = self.close_queue_capacity();
        validate_parallel_queue_invariant(self.parallel_merkle_enabled, close_queue_capacity)?;

        // Spawn the finalizer pipeline: serial (default) or parallel merkle.
        // In parallel mode, the finalizer streams per-block root jobs to a bounded
        // worker pool while preserving ordered commit/write semantics.
        let (close_queue_handle, finalizer_task_handle) = if self.parallel_merkle_enabled {
            let (close_queue_handle, finalizer_task_handle) = FinalizerHandle::spawn_parallel(
                close_queue_capacity,
                self.parallel_merkle_root_workers,
                self.metrics.clone(),
                Self::compute_close_payload_parallel_root,
                Self::execute_close_payload_parallel_precomputed_job,
            );
            tracing::info!(
                "initialized_finalizer_runtime mode=parallel_merkle queue_capacity={} configured_max_inflight={} configured_capacity={} parallel_merkle={} parallel_merkle_root_workers={} parallel_merkle_compare_sequential={}",
                close_queue_capacity,
                self.close_queue_capacity,
                close_queue_handle.configured_capacity(),
                true,
                self.parallel_merkle_root_workers,
                self.parallel_merkle_compare_sequential
            );
            (close_queue_handle, finalizer_task_handle)
        } else {
            let (close_queue_handle, finalizer_task_handle) =
                FinalizerHandle::spawn(close_queue_capacity, self.metrics.clone(), Self::execute_close_payload_batch);
            tracing::info!(
                "initialized_finalizer_runtime mode=serial queue_capacity={} configured_max_inflight={} configured_capacity={}",
                close_queue_capacity,
                self.close_queue_capacity,
                close_queue_handle.configured_capacity()
            );
            (close_queue_handle, finalizer_task_handle)
        };

        // Batcher task is handled in a separate tokio task.
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        // Clone ctx to check for cancellation in the main loop
        let mut batcher_task = AbortOnDrop::spawn(
            Batcher::new(
                self.backend.clone(),
                self.mempool.clone(),
                self.metrics.clone(),
                self.l1_client.clone(),
                ctx,
                batch_sender,
                bypass_tx_input,
                self.mempool_intake_rx.clone(),
            )
            .run(),
        );

        // Track shutdown state: both batcher and executor must complete before shutdown finishes.
        // Both tasks only complete during shutdown scenarios (cancellation, error, or panic).
        let mut batcher_completed = false;
        let mut end_final_block_received = false; // Track if EndFinalBlock has been processed (executor completed with block)
        let mut executor_stopped = false; // Track if executor.stop has been received (oneshot - can only poll once)
        let mut batcher_error: Option<anyhow::Error> = None; // Store batcher error to return after graceful shutdown

        // Main loop: handles normal operation and graceful shutdown.
        // Captures loop outcome; finalizer drain runs unconditionally after.
        let loop_result: anyhow::Result<()> = loop {
            tokio::select! {
                // Path 1: Batcher task completed (cancellation, error, or channel closure)
                res = &mut batcher_task, if !batcher_completed => {
                    batcher_completed = true;
                    match res {
                        Ok(()) => tracing::debug!("Batcher task completed normally"),
                        Err(e) => {
                            let error = e.context("In batcher task");
                            tracing::warn!("Batcher task errored: {error:?}");
                            batcher_error = Some(error);
                            if self.backend.has_preconfirmed_block() {
                                tracing::warn!("Batcher errored with preconfirmed block, attempting graceful shutdown");
                            }
                        }
                    }
                }

                // Path 2: Executor replies (EndBlock for normal operation, EndFinalBlock for shutdown)
                Some(reply) = executor.replies.recv() => {
                    let is_end_final_block = matches!(reply, ExecutorMessage::EndFinalBlock(_));
                    if let Err(e) = self.process_reply(reply, &close_queue_handle)
                        .await
                        .context("Processing reply from executor thread")
                    {
                        break Err(e);
                    }
                    // Mark executor as completed only after processing EndFinalBlock
                    if is_end_final_block {
                        end_final_block_received = true;
                        tracing::debug!("EndFinalBlock processed, executor completed");
                    }
                }

                completion_res = async {
                    let (_block_n, rx) = self.pending_completions.front_mut().expect("checked non-empty");
                    rx.await
                }, if !self.pending_completions.is_empty() => {
                    let (expected_block_n, _rx) = self.pending_completions.pop_front().expect("pending completion exists");
                    let completion = completion_res
                        .context("Close queue worker dropped completion channel")??;
                    self.metrics.close_queue_dequeued_total.add(1, &[]);
                    self.metrics.close_queue_depth.record(close_queue_handle.current_depth() as u64, &[]);
                    tracing::debug!(
                        "close_block_complete block_number={} expected_block_n={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={} parallel_merkle={}",
                        completion.block_n,
                        expected_block_n,
                        close_queue_handle.current_depth(),
                        close_queue_handle.configured_capacity(),
                        close_queue_handle.current_in_flight(),
                        self.pending_completions.len(),
                        self.parallel_merkle_enabled
                    );
                    if completion.block_n != expected_block_n {
                        break Err(anyhow::anyhow!(
                            "Out-of-order close completion: expected #{expected_block_n}, got #{}",
                            completion.block_n
                        ));
                    }
                    if self.parallel_merkle_enabled && self.is_boundary_block(completion.block_n) {
                        prune_diffs_since_snapshot(&mut self.diffs_since_snapshot, completion.block_n);
                    }
                    if let Some(status) = self.backend.replay_boundary_mark_closed(completion.block_n) {
                        if !status.boundary_met {
                            tracing::warn!(
                                "replay_boundary_closed_without_match block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} mismatch={:?}",
                                status.block_n,
                                status.expected_tx_count,
                                status.executed_tx_count,
                                status.dispatched_tx_count,
                                status.reached_last_tx_hash,
                                status.mismatch
                            );
                        }
                    }
                    self.send_state_notification(BlockProductionStateNotification::ClosedBlock { block_n: completion.block_n });
                    self.record_block_stage_metrics();
                }

                // Path 3: Executor thread stopped (normal completion or panic)
                // This fires when executor exits. EndFinalBlock should have been emitted by executor
                // (executor always sends EndFinalBlock during shutdown - Some(summary) if block exists, None if no block).
                // Guard: oneshot channel can only be polled once - polling after completion causes panic.
                res = executor.stop.recv(), if !executor_stopped => {
                    executor_stopped = true;
                    if let Err(e) = res.context("In executor thread") {
                        break Err(e);
                    }
                }
            }

            // Exit conditions (checked after each select iteration):
            // Shutdown is complete when batcher completed AND EndFinalBlock was processed.
            // Executor always sends EndFinalBlock during shutdown (Some(summary) if block exists, None if no block).
            if batcher_completed && end_final_block_received && self.pending_completions.is_empty() {
                tracing::debug!("Shutdown complete: batcher completed, EndFinalBlock processed");
                let shutdown_result = batcher_error
                    .map(|e| {
                        tracing::warn!("Shutdown completed but batcher had error: {e:?}");
                        Err(e)
                    })
                    .unwrap_or(Ok(()));
                break shutdown_result;
            }
        };

        // Unconditional finalizer drain: always drop sender and join worker,
        // regardless of whether the main loop exited normally or via error.
        drop(close_queue_handle);
        let finalizer_result = finalizer_task_handle.join().await;

        // Shutdown drain path: apply the same boundary-prune rule for any queued completions that
        // may still be present when the main loop exits via an error path.
        while let Some((expected_block_n, rx)) = self.pending_completions.pop_front() {
            match rx.await {
                Ok(Ok(completion)) => {
                    if completion.block_n == expected_block_n
                        && self.parallel_merkle_enabled
                        && self.is_boundary_block(completion.block_n)
                    {
                        prune_diffs_since_snapshot(&mut self.diffs_since_snapshot, completion.block_n);
                    }
                    if let Some(status) = self.backend.replay_boundary_mark_closed(completion.block_n) {
                        if !status.boundary_met {
                            tracing::warn!(
                                "replay_boundary_closed_without_match block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} mismatch={:?}",
                                status.block_n,
                                status.expected_tx_count,
                                status.executed_tx_count,
                                status.dispatched_tx_count,
                                status.reached_last_tx_hash,
                                status.mismatch
                            );
                        }
                    }
                }
                Ok(Err(err)) => {
                    tracing::warn!("Shutdown drain: close completion for block #{} failed: {err:#}", expected_block_n);
                }
                Err(err) => {
                    tracing::warn!("Shutdown drain: completion channel dropped for block #{}: {err}", expected_block_n);
                }
            }
            self.record_block_stage_metrics();
        }

        // Explicitly clear in-memory pipeline state after shutdown drain so reorg/shutdown
        // paths do not retain stale per-block runtime data.
        self.pending_completions.clear();
        self.diffs_since_snapshot.clear();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: self.backend.latest_confirmed_block_n() });
        self.record_block_stage_metrics();

        // Compose errors: if both loop and finalizer failed, attach finalizer
        // error as context on the primary error so neither is lost.
        match (loop_result, finalizer_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(finalizer_err)) => Err(finalizer_err.context("In finalizer worker")),
            (Err(primary_err), Ok(())) => Err(primary_err),
            (Err(primary_err), Err(finalizer_err)) => {
                tracing::warn!("Finalizer worker also errored during shutdown: {finalizer_err:?}");
                Err(primary_err.context(format!("Additionally, finalizer worker errored: {finalizer_err:#}")))
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::BlockProductionStateNotification;
    use crate::{metrics::BlockProductionMetrics, BlockProductionTask};
    use blockifier::bouncer::{BouncerConfig, BouncerWeights};
    use mc_db::{preconfirmed::PreconfirmedBlock, MadaraBackend, MadaraBackendConfig};
    use mc_devnet::{
        Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
    };
    use mc_mempool::{Mempool, MempoolConfig};
    use mc_settlement_client::L1ClientMock;
    use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
    use mp_block::header::PreconfirmedHeader;
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use mp_receipt::{Event, ExecutionResult};
    use mp_rpc::v0_9_0::{
        BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, DaMode,
        InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
    };
    use mp_state_update::StateDiff;
    use mp_transactions::compute_hash::calculate_contract_address;
    use mp_transactions::IntoStarknetApiExt;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
    use mp_utils::service::ServiceContext;
    use mp_utils::AbortOnDrop;
    use starknet_core::utils::get_selector_from_name;
    use starknet_types_core::felt::Felt;
    use std::{sync::Arc, time::Duration};

    type TxFixtureInfo = (Transaction, mp_receipt::TransactionReceipt);

    #[rstest::rstest]
    #[case::parallel_below_minimum(true, 9, false)]
    #[case::parallel_minimum(true, 10, true)]
    #[case::serial_any_capacity(false, 1, true)]
    fn queue_invariant_matrix(#[case] parallel: bool, #[case] capacity: usize, #[case] expect_ok: bool) {
        let result = super::validate_parallel_queue_invariant(parallel, capacity);
        assert_eq!(result.is_ok(), expect_ok);
        if !expect_ok {
            let msg = format!("{:#}", result.expect_err("must fail"));
            assert!(msg.contains("QueueInvariantViolated"));
        }
    }

    #[test]
    fn preconfirmed_runahead_is_accepted_before_previous_close_completes() {
        let backend = MadaraBackend::open_for_testing_with_config(
            Arc::new(ChainConfig::madara_devnet()),
            MadaraBackendConfig { save_preconfirmed: true, ..Default::default() },
        );

        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 0, ..Default::default() }))
            .expect("creating preconfirmed block #0 should succeed");
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new(PreconfirmedHeader { block_number: 1, ..Default::default() }))
            .expect("creating preconfirmed block #1 should succeed while #0 is still externally visible");

        let head = backend.chain_head_state();
        assert_eq!(head.confirmed_tip, None);
        assert_eq!(head.external_preconfirmed_tip, Some(0));
        assert_eq!(head.internal_preconfirmed_tip, Some(1));
    }

    fn empty_state_diff() -> StateDiff {
        StateDiff {
            storage_diffs: vec![],
            old_declared_contracts: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
            migrated_compiled_classes: vec![],
        }
    }

    #[rstest::rstest]
    #[case::prune_nothing(vec![(11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
    #[case::prune_prefix(vec![(10, empty_state_diff()), (11, empty_state_diff()), (12, empty_state_diff())], 10, vec![11, 12])]
    #[case::prune_all(vec![(10, empty_state_diff())], 10, vec![])]
    fn boundary_prune_matrix(
        #[case] mut input: Vec<(u64, StateDiff)>,
        #[case] completed_block_n: u64,
        #[case] expected_blocks: Vec<u64>,
    ) {
        super::prune_diffs_since_snapshot(&mut input, completed_block_n);
        let remaining_blocks = input.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(remaining_blocks, expected_blocks);
    }

    #[rstest::rstest]
    #[case::from_empty_base(vec![(0, empty_state_diff()), (1, empty_state_diff()), (2, empty_state_diff())], None, 2, 3)]
    #[case::from_snapshot_floor(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(89), 92, 3)]
    #[case::skip_pruned_prefix(vec![(90, empty_state_diff()), (91, empty_state_diff()), (92, empty_state_diff())], Some(90), 92, 2)]
    fn collect_diffs_for_root_from_base_ok(
        #[case] input: Vec<(u64, StateDiff)>,
        #[case] base_block_n: Option<u64>,
        #[case] target_block_n: u64,
        #[case] expected_len: usize,
    ) {
        let collected = super::collect_diffs_for_root_from_base(&input, base_block_n, target_block_n)
            .expect("diff span should be contiguous");
        assert_eq!(collected.len(), expected_len);
    }

    #[test]
    fn collect_diffs_for_root_from_base_rejects_gap() {
        let input = vec![(90, empty_state_diff()), (92, empty_state_diff())];
        let err = super::collect_diffs_for_root_from_base(&input, Some(89), 92).expect_err("gap must fail");
        let msg = format!("{err:#}");
        assert!(msg.contains("Missing tracked state diff for block #91"));
    }

    #[rstest::fixture]
    fn backend() -> Arc<MadaraBackend> {
        MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()))
    }

    #[rstest::fixture]
    fn bouncer_weights() -> BouncerWeights {
        // The bouncer weights values are configured in such a way
        // that when loaded, the block will close after one transaction
        // is added to it, to test the pending tick closing the block
        BouncerWeights { sierra_gas: starknet_api::execution_resources::GasAmount(1000000), ..BouncerWeights::max() }
    }

    pub struct DevnetSetup {
        pub backend: Arc<MadaraBackend>,
        pub metrics: Arc<BlockProductionMetrics>,
        pub mempool: Arc<Mempool>,
        pub tx_validator: Arc<TransactionValidator>,
        pub contracts: DevnetKeys,
        pub l1_client: L1ClientMock,
    }

    impl DevnetSetup {
        pub fn block_prod_task(&mut self) -> BlockProductionTask {
            BlockProductionTask::new(
                self.backend.clone(),
                self.mempool.clone(),
                self.metrics.clone(),
                Arc::new(self.l1_client.clone()),
                false, /* mempool_paused = false */
                false, /* no_charge_fee = false */
            )
        }
    }

    #[rstest::fixture]
    pub async fn devnet_setup(
        #[default(Duration::from_secs(30))] block_time: Duration,
        #[default(false)] use_bouncer_weights: bool,
    ) -> DevnetSetup {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
        let mut genesis = ChainGenesisDescription::base_config().unwrap();
        let contracts = genesis.add_devnet_contracts(10).unwrap();

        let chain_config: Arc<ChainConfig> = if use_bouncer_weights {
            let bouncer_weights = bouncer_weights();

            Arc::new(ChainConfig {
                block_time,
                bouncer_config: BouncerConfig {
                    block_max_capacity: bouncer_weights,
                    builtin_weights: Default::default(),
                    blake_weight: Default::default(),
                },
                ..ChainConfig::madara_devnet()
            })
        } else {
            Arc::new(ChainConfig { block_time, ..ChainConfig::madara_devnet() })
        };

        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        backend.set_l1_gas_quote_for_testing();
        genesis.build_and_store(&backend).await.unwrap();

        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
        let tx_validator = Arc::new(TransactionValidator::new(
            Arc::clone(&mempool) as _,
            Arc::clone(&backend),
            TransactionValidatorConfig::default(), /* disable_fee = false, disable_validation = false */
        ));

        DevnetSetup {
            backend,
            mempool,
            metrics: Arc::new(BlockProductionMetrics::register()),
            tx_validator,
            contracts,
            l1_client: L1ClientMock::new(),
        }
    }

    #[rstest::fixture]
    pub fn tx_invoke_v0(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
        (
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0 { contract_address, ..Default::default() },
            )),
            mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    pub fn tx_l1_handler(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
        (
            mp_transactions::Transaction::L1Handler(mp_transactions::L1HandlerTransaction {
                contract_address,
                ..Default::default()
            }),
            mp_receipt::TransactionReceipt::L1Handler(mp_receipt::L1HandlerTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    fn tx_declare_v0(#[default(Felt::ZERO)] sender_address: Felt) -> TxFixtureInfo {
        (
            mp_transactions::Transaction::Declare(mp_transactions::DeclareTransaction::V0(
                mp_transactions::DeclareTransactionV0 { sender_address, ..Default::default() },
            )),
            mp_receipt::TransactionReceipt::Declare(mp_receipt::DeclareTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    pub fn tx_deploy() -> TxFixtureInfo {
        (
            mp_transactions::Transaction::Deploy(mp_transactions::DeployTransaction::default()),
            mp_receipt::TransactionReceipt::Deploy(mp_receipt::DeployTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    pub fn tx_deploy_account() -> TxFixtureInfo {
        (
            mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V1(
                mp_transactions::DeployAccountTransactionV1::default(),
            )),
            mp_receipt::TransactionReceipt::DeployAccount(mp_receipt::DeployAccountTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    pub fn converted_class_legacy(#[default(Felt::ZERO)] class_hash: Felt) -> mp_class::ConvertedClass {
        mp_class::ConvertedClass::Legacy(mp_class::LegacyConvertedClass {
            class_hash,
            info: mp_class::LegacyClassInfo {
                contract_class: Arc::new(mp_class::CompressedLegacyContractClass {
                    program: vec![],
                    entry_points_by_type: mp_class::LegacyEntryPointsByType {
                        constructor: vec![],
                        external: vec![],
                        l1_handler: vec![],
                    },
                    abi: None,
                }),
            },
        })
    }

    #[rstest::fixture]
    pub fn converted_class_sierra(
        #[default(Felt::ZERO)] class_hash: Felt,
        #[default(Felt::ZERO)] compiled_class_hash: Felt,
    ) -> mp_class::ConvertedClass {
        mp_class::ConvertedClass::Sierra(mp_class::SierraConvertedClass {
            class_hash,
            info: mp_class::SierraClassInfo {
                contract_class: Arc::new(mp_class::FlattenedSierraClass {
                    sierra_program: vec![],
                    contract_class_version: "".to_string(),
                    entry_points_by_type: mp_class::EntryPointsByType {
                        constructor: vec![],
                        external: vec![],
                        l1_handler: vec![],
                    },
                    abi: "".to_string(),
                }),
                compiled_class_hash: Some(compiled_class_hash),
                compiled_class_hash_v2: None,
            },
            compiled: Arc::new(mp_class::CompiledSierra("".to_string())),
        })
    }

    pub fn make_declare_tx(
        contract: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        nonce: Felt,
    ) -> BroadcastedDeclareTxn {
        let sierra_class: starknet_core::types::contract::SierraClass =
            serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
        let flattened_class: mp_class::FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();

        // Use BLAKE hash (v2) for v0.14.1+ compatibility
        let hashes = flattened_class.compile_to_casm_with_hashes().unwrap();
        let compiled_contract_class_hash = hashes.blake_hash;

        let mut declare_txn: BroadcastedDeclareTxn = BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
            sender_address: contract.address,
            compiled_class_hash: compiled_contract_class_hash,
            // this field will be filled below
            signature: vec![].into(),
            nonce,
            contract_class: flattened_class.into(),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
                l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let (api_tx, _class) = BroadcastedTxn::Declare(declare_txn.clone())
            .into_starknet_api(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
            )
            .unwrap();
        let signature = contract.secret.sign(&api_tx.tx_hash().0).unwrap();

        let tx_signature = match &mut declare_txn {
            BroadcastedDeclareTxn::V1(tx) => &mut tx.signature,
            BroadcastedDeclareTxn::V2(tx) => &mut tx.signature,
            BroadcastedDeclareTxn::V3(tx) => &mut tx.signature,
            _ => unreachable!("the declare tx is not query only"),
        };
        *tx_signature = vec![signature.r, signature.s].into();
        declare_txn
    }

    pub async fn sign_and_add_declare_tx(
        contract: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        validator: &Arc<TransactionValidator>,
        nonce: Felt,
    ) -> ClassAndTxnHash {
        validator
            .submit_declare_transaction(make_declare_tx(contract, backend, nonce))
            .await
            .expect("Should accept the transaction")
    }

    pub fn make_invoke_tx(
        contract_sender: &DevnetPredeployedContract,
        multicall: Multicall,
        backend: &Arc<MadaraBackend>,
        nonce: Felt,
    ) -> BroadcastedInvokeTxn {
        let mut invoke_txn: BroadcastedInvokeTxn = BroadcastedInvokeTxn::V3(InvokeTxnV3 {
            sender_address: contract_sender.address,
            calldata: multicall.flatten().collect::<Vec<_>>().into(),
            // this field will be filled below
            signature: vec![].into(),
            nonce,
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
                l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let (api_tx, _classes) = BroadcastedTxn::Invoke(invoke_txn.clone())
            .into_starknet_api(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
            )
            .unwrap();
        let signature = contract_sender.secret.sign(&api_tx.tx_hash()).unwrap();

        let tx_signature = match &mut invoke_txn {
            BroadcastedInvokeTxn::V0(tx) => &mut tx.signature,
            BroadcastedInvokeTxn::V1(tx) => &mut tx.signature,
            BroadcastedInvokeTxn::V3(tx) => &mut tx.signature,
            _ => unreachable!("the invoke tx is not query only"),
        };
        *tx_signature = vec![signature.r, signature.s].into();

        invoke_txn
    }

    pub fn make_udc_call(
        contract_sender: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        nonce: Felt,
        class_hash: Felt,
        constructor_calldata: &[Felt],
    ) -> (Felt, BroadcastedInvokeTxn) {
        let contract_address = calculate_contract_address(
            /* salt */ Felt::ZERO,
            class_hash,
            constructor_calldata,
            /* deployer_address */ Felt::ZERO,
        );

        (
            contract_address,
            make_invoke_tx(
                contract_sender,
                Multicall::default().with(Call {
                    to: UDC_CONTRACT_ADDRESS,
                    selector: Selector::from("deployContract"),
                    calldata: [
                        class_hash,
                        /* salt */ Felt::ZERO,
                        /* unique */ Felt::ZERO,
                        constructor_calldata.len().into(),
                    ]
                    .into_iter()
                    .chain(constructor_calldata.iter().copied())
                    .collect(),
                }),
                backend,
                nonce,
            ),
        )
    }

    pub async fn sign_and_add_invoke_tx(
        contract_sender: &DevnetPredeployedContract,
        contract_receiver: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        validator: &Arc<TransactionValidator>,
        nonce: Felt,
    ) {
        let erc20_contract_address =
            Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

        let tx = make_invoke_tx(
            contract_sender,
            Multicall::default().with(Call {
                to: erc20_contract_address,
                selector: Selector::from("transfer"),
                calldata: vec![contract_receiver.address, (9_999u128 * 1_000_000_000_000_000_000).into(), Felt::ZERO],
            }),
            backend,
            nonce,
        );

        validator.submit_invoke_transaction(tx).await.expect("Should accept the transaction");
    }

    //
    // This test verifies that when Madara restarts with a preconfirmed block, `close_preconfirmed_block_if_exists`
    // correctly re-executes transactions and produces the same global state root, state diff, and receipts as the original
    // execution. This ensures correctness of the restart recovery mechanism.
    //
    // # Test Process
    //
    // **Phase 1: Normal Block Production**
    // 1. Creates a block with various transaction types (invoke, declare, deploy, L1 handler)
    // 2. Closes the block normally and captures:
    //    - `global_state_root`
    //    - `state_diff`
    //    - `header` information
    //    - Executed transactions
    //
    // # Transaction Types Tested
    // - **Invoke transactions**: Standard contract calls
    // - **Declare transactions**: Class declarations
    // - **Deploy transactions**: Contract deployments via UDC
    // - **L1 handler transactions**: L1 to L2 messages with `paid_fee_on_l1`
    //
    // # Key Assertions
    //
    // - Global state root must match exactly (ensures state consistency)
    // - State diff must match (values are the same, order may differ)
    // - Header fields must match the preconfirmed block (timestamp, gas_prices, etc.)
    // - All transactions must match
    // - All receipts must match exactly (ensures execution results are identical)
    //
    // # Important Notes
    //
    // - Uses two separate `DevnetSetup` fixtures to ensure clean state isolation
    // - State diffs are sorted before comparison to handle ordering differences
    // - The test verifies that `paid_fee_on_l1` is preserved for L1 handler transactions
    // - The test ensures that re-execution produces deterministic results
    #[rstest::rstest]
    #[timeout(Duration::from_secs(100))]
    #[tokio::test]
    async fn test_close_preconfirmed_block_reexecution_matches_normal_closing(
        #[future]
        #[from(devnet_setup)]
        original_devnet_setup: DevnetSetup,
        #[future]
        #[from(devnet_setup)]
        restart_devnet_setup: DevnetSetup,
    ) {
        // used for phase 1, where we close the block and note down its
        // global_state_root, state_diff, and header info
        let mut original_devnet_setup = original_devnet_setup.await;

        // use for phase 2, where we compare the state of the block after re-execution with the state of the block before re-execution
        let mut restart_devnet_setup = restart_devnet_setup.await;

        // --------------------------------------------------------------
        // | PHASE 1: Close the block and note down its state.          |
        // --------------------------------------------------------------

        // Step 1: Create a block normally with transactions in the original backend
        assert!(original_devnet_setup.mempool.is_empty().await);

        // Helper function to create and execute transactions for testing
        async fn create_and_execute_transactions(setup: &DevnetSetup) -> Felt {
            // 1. Declare a contract
            let declare_res =
                sign_and_add_declare_tx(&setup.contracts.0[0], &setup.backend, &setup.tx_validator, Felt::ZERO).await;

            // 2. Deploy contract through UDC
            let (contract_address, deploy_tx) = make_udc_call(
                &setup.contracts.0[0],
                &setup.backend,
                /* nonce */ Felt::ONE,
                declare_res.class_hash,
                /* calldata (pubkey) */ &[Felt::TWO],
            );
            setup.tx_validator.submit_invoke_transaction(deploy_tx).await.unwrap();

            // 3. Invoke transaction
            sign_and_add_invoke_tx(
                &setup.contracts.0[0],
                &setup.contracts.0[1],
                &setup.backend,
                &setup.tx_validator,
                Felt::TWO, // nonce after declare (ZERO) and deploy (ONE)
            )
            .await;

            // 4. Declare transaction (for a different contract)
            sign_and_add_declare_tx(
                &setup.contracts.0[2],
                &setup.backend,
                &setup.tx_validator,
                Felt::ZERO, // Different account, so nonce starts at ZERO
            )
            .await;

            // 5. Another invoke transaction
            sign_and_add_invoke_tx(
                &setup.contracts.0[1],
                &setup.contracts.0[3],
                &setup.backend,
                &setup.tx_validator,
                Felt::ZERO, // Different account, so nonce starts at ZERO
            )
            .await;

            // 6. Add L1 handler transaction
            let paid_fee_on_l1 = 128328u128;
            setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
                L1HandlerTransaction {
                    version: Felt::ZERO,
                    nonce: 55, // core contract nonce
                    contract_address,
                    entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
                    calldata: vec![
                        /* from_address */ Felt::THREE,
                        /* arg1 */ Felt::ONE,
                        /* arg2 */ Felt::TWO,
                    ]
                    .into(),
                },
                paid_fee_on_l1,
            ));

            contract_address
        }

        // Add various transaction types to mempool to test re-execution handles all types correctly
        // All transactions will be in a single block
        let _contract_address = create_and_execute_transactions(&original_devnet_setup).await;

        assert!(!original_devnet_setup.mempool.is_empty().await);

        // Run block production to create and close a block with all transactions
        let mut block_production_task = original_devnet_setup.block_prod_task();
        let mut notifications = block_production_task.subscribe_state_notifications();
        let control = block_production_task.handle();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // Wait for batch to be executed
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Manually close the block
        control.close_block().await.unwrap();
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 1 }
        ));

        // Step 2: Capture global_state_root, state_diff, and header info from closed block
        let block_number = original_devnet_setup.backend.latest_confirmed_block_n().unwrap();
        let original_block = original_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
        let original_block_info = original_block.get_block_info().unwrap();
        let expected_global_state_root = original_block_info.header.global_state_root;
        let expected_state_diff = original_block.get_state_diff().unwrap();
        let executed_transactions = original_block.get_executed_transactions(..).unwrap();

        // --------------------------------------------------------------
        // | PHASE 2: Re-execute the block and note down its state.    |
        // --------------------------------------------------------------
        //
        // We'll add them in the same order using the same helper functions
        // All transactions will be in a single block
        // This ensures they're executed in the same context (clean genesis state)
        assert!(restart_devnet_setup.mempool.is_empty().await);

        // Create the same transactions using the helper function
        let _restart_contract_address = create_and_execute_transactions(&restart_devnet_setup).await;

        assert!(!restart_devnet_setup.mempool.is_empty().await);

        // Step 4: Run block production to execute transactions and add them to preconfirmed block
        // Use a very long block_time to prevent auto-closing, then stop manually after batch execution
        let mut restart_block_production_task = restart_devnet_setup.block_prod_task();
        let mut restart_notifications = restart_block_production_task.subscribe_state_notifications();
        let restart_task = AbortOnDrop::spawn(async move {
            restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
        });

        // Wait for batch to be executed (transactions added to preconfirmed block)
        assert_eq!(restart_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Fetch preconfirmed block view BEFORE dropping the task to avoid race conditions
        let preconfirmed_view = restart_devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
        assert_eq!(preconfirmed_view.num_executed_transactions(), executed_transactions.len());
        let restart_preconfirmed_block = preconfirmed_view.block();

        // Stop the task before it closes the block (drop the AbortOnDrop which will abort the task)
        drop(restart_task);

        // Give it a moment to finish current operations
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify preconfirmed block still exists with transactions and no confirmed blocks yet
        assert!(restart_devnet_setup.backend.has_preconfirmed_block());
        assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(0));

        // adding some delay to see if block_timestamp would differ in the reexecution or not
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Step 5: Now call close_preconfirmed_block_if_exists to re-execute and close the preconfirmed block
        let mut reexec_block_production_task = restart_devnet_setup.block_prod_task();
        reexec_block_production_task.close_preconfirmed_block_if_exists().await.unwrap();

        // Step 6: Verify results match
        assert!(!restart_devnet_setup.backend.has_preconfirmed_block());
        assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(block_number));

        let reexecuted_block_info =
            restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap().get_block_info().unwrap();

        // Verify the header fields match the pre-execution pre-confirmed block's header
        assert_eq!(restart_preconfirmed_block.header.block_timestamp, reexecuted_block_info.header.block_timestamp);
        assert_eq!(restart_preconfirmed_block.header.protocol_version, reexecuted_block_info.header.protocol_version);
        assert_eq!(restart_preconfirmed_block.header.l1_da_mode, reexecuted_block_info.header.l1_da_mode);
        assert_eq!(restart_preconfirmed_block.header.gas_prices, reexecuted_block_info.header.gas_prices);
        assert_eq!(restart_preconfirmed_block.header.sequencer_address, reexecuted_block_info.header.sequencer_address);
        assert_eq!(restart_preconfirmed_block.header.block_number, reexecuted_block_info.header.block_number);

        let reexecuted_block = restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
        let reexecuted_block_info = reexecuted_block.get_block_info().unwrap();
        let actual_global_state_root = reexecuted_block_info.header.global_state_root;
        let mut actual_state_diff = reexecuted_block.get_state_diff().unwrap();
        let mut expected_state_diff_sorted = expected_state_diff.clone();

        // Sort both state diffs to normalize ordering before comparison
        actual_state_diff.sort();
        expected_state_diff_sorted.sort();

        // Verify global state root matches
        assert_eq!(
            actual_global_state_root, expected_global_state_root,
            "Global state root should match between normal execution and re-execution"
        );

        // Verify state diff matches (after sorting to ignore ordering differences)
        assert_eq!(
            actual_state_diff, expected_state_diff_sorted,
            "State diff should match between normal execution and re-execution (values are the same, only order may differ)"
        );

        // Verify transactions match
        let reexecuted_transactions = reexecuted_block.get_executed_transactions(..).unwrap();
        assert_eq!(reexecuted_transactions, executed_transactions, "Transactions should match");

        // Verify receipts match - re-execution should produce identical receipts
        assert_eq!(executed_transactions.len(), reexecuted_transactions.len(), "Number of transactions should match");
        for (i, (original_tx, reexecuted_tx)) in
            executed_transactions.iter().zip(reexecuted_transactions.iter()).enumerate()
        {
            assert_eq!(
                original_tx.receipt.transaction_hash(),
                reexecuted_tx.receipt.transaction_hash(),
                "Receipt transaction hash should match for transaction {}",
                i
            );
            assert_eq!(
                original_tx.receipt,
                reexecuted_tx.receipt,
                "Receipt should match exactly for transaction {} (hash: {:#x})",
                i,
                original_tx.receipt.transaction_hash()
            );
        }
    }

    // This test makes sure that the preconfirmed tick closes the block
    // if the bouncer capacity is reached
    #[ignore] // FIXME: this test is complicated by the fact validation / actual execution fee may differ a bit. Ignore for now.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_bouncer_cap_reached_closes_block(
        #[future]
        // Use a very very long block time (longer than the test timeout).
        #[with(Duration::from_secs(10000000), true)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(devnet_setup.mempool.is_empty().await);
        sign_and_add_invoke_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.contracts.0[1],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        sign_and_add_invoke_tx(
            &devnet_setup.contracts.0[1],
            &devnet_setup.contracts.0[2],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        sign_and_add_invoke_tx(
            &devnet_setup.contracts.0[2],
            &devnet_setup.contracts.0[3],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        assert!(!devnet_setup.mempool.is_empty().await);

        let mut block_production_task = devnet_setup.block_prod_task();
        // The BouncerConfig is set up with amounts (100000) that should limit
        // the block size in a way that the pending tick on this task
        // closes the block
        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        tokio::time::sleep(Duration::from_secs(5)).await;

        tracing::debug!("{:?}", devnet_setup.backend.block_view_on_latest().map(|l| l.get_executed_transactions(..)));
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 1 }
        ));
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 2 }
        ));
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        let closed_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
        let closed_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
        let preconfirmed_3 = devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
        assert_eq!(preconfirmed_3.block_number(), 3);
        assert_eq!(closed_1.get_executed_transactions(..).unwrap().len(), 1);
        // rolled over to next block.
        assert_eq!(closed_2.get_executed_transactions(..).unwrap().len(), 1);
        // rolled over to next block.
        // last block should not be closed though.
        assert_eq!(preconfirmed_3.get_executed_transactions(..).len(), 1);
        assert!(devnet_setup.mempool.is_empty().await);
    }

    // This test makes sure that the block time tick correctly
    // adds the transaction to the preconfirmed block, closes it
    // and creates a new empty preconfirmed block
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_block_time_tick_closes_block(
        #[future]
        #[with(Duration::from_secs(2), true)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        let mut block_production_task = devnet_setup.block_prod_task();

        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // The block should be closed after 3s.
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 1 }
        ));

        let view = devnet_setup.backend.block_view_on_last_confirmed().unwrap();

        assert_eq!(view.block_number(), 1);
        assert_eq!(view.get_executed_transactions(..).unwrap(), []);
    }

    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    async fn test_l1_handler_tx(
        #[future]
        #[with(Duration::from_secs(3000000000), false)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;
        let mut block_production_task = devnet_setup.block_prod_task();

        let mut notifications = block_production_task.subscribe_state_notifications();
        let control = block_production_task.handle();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // Declare the contract class.
        let res = sign_and_add_declare_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            /* nonce */ Felt::ZERO,
        )
        .await;

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        assert_eq!(
            devnet_setup
                .backend
                .block_view_on_current_preconfirmed()
                .unwrap()
                .get_executed_transaction(0)
                .unwrap()
                .receipt
                .execution_result(),
            ExecutionResult::Succeeded
        );
        control.close_block().await.unwrap();
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 1 }
        ));

        // Deploy contract through UDC.

        let (contract_address, tx) = make_udc_call(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            /* nonce */ Felt::ONE,
            res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        devnet_setup.tx_validator.submit_invoke_transaction(tx).await.unwrap();

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        assert_eq!(
            devnet_setup
                .backend
                .block_view_on_current_preconfirmed()
                .unwrap()
                .get_executed_transaction(0)
                .unwrap()
                .receipt
                .execution_result(),
            ExecutionResult::Succeeded
        );

        control.close_block().await.unwrap();
        assert!(matches!(
            notifications.recv().await.unwrap(),
            BlockProductionStateNotification::ClosedBlock { block_n: 2 }
        ));

        // Mock the l1 message, block prod should pick it up.

        devnet_setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 55, // core contract nonce
                contract_address,
                entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
                calldata: vec![
                    /* from_address */ Felt::THREE,
                    /* arg1 */ Felt::ONE,
                    /* arg2 */ Felt::TWO,
                ]
                .into(),
            },
            /* paid_fee_on_l1 */ 128328,
        ));

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        let receipt = devnet_setup
            .backend
            .block_view_on_current_preconfirmed()
            .unwrap()
            .get_executed_transaction(0)
            .unwrap()
            .receipt;
        assert_eq!(receipt.execution_result(), ExecutionResult::Succeeded);
        tracing::info!("Events = {:?}", receipt.events());
        assert_eq!(receipt.events().len(), 1);

        assert_eq!(
            receipt.events()[0],
            Event {
                from_address: contract_address,
                keys: vec![get_selector_from_name("CalledFromL1").unwrap()],
                data: vec![/* from_address */ Felt::THREE, /* arg1 */ Felt::ONE, /* arg2 */ Felt::TWO]
            }
        );
    }

    /// Verifies that re-execution uses the saved `no_charge_fee` value.
    ///
    /// # Flow
    /// 1. **Initial**: `no_charge_fee = true`. Exec tx, stop before closing. Saved: `true`.
    /// 2. **Restart**: `no_charge_fee = false`.
    /// 3. **Re-execution**: Uses saved `true` value. Receipts match.
    /// 4. **Post**: Config updates to `false` for next block.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(100))]
    #[tokio::test]
    async fn test_reexecution_uses_saved_no_charge_fee_value(
        #[future]
        #[from(devnet_setup)]
        original_devnet_setup: DevnetSetup,
    ) {
        let original_devnet_setup = original_devnet_setup.await;

        // Phase 1: Initial execution with no_charge_fee = true
        let initial_no_charge_fee = true;
        assert!(original_devnet_setup.mempool.is_empty().await);

        // Create a transaction validator that matches our no_charge_fee setting.
        // This ensures transactions are validated with charge_fee = !no_charge_fee.
        // Without this, transactions would be validated with charge_fee = true (default),
        // causing a mismatch between validation and execution.
        let tx_validator_with_no_fee = Arc::new(TransactionValidator::new(
            Arc::clone(&original_devnet_setup.mempool) as _,
            Arc::clone(&original_devnet_setup.backend),
            TransactionValidatorConfig { disable_validation: false, disable_fee: initial_no_charge_fee },
        ));

        sign_and_add_invoke_tx(
            &original_devnet_setup.contracts.0[0],
            &original_devnet_setup.contracts.0[1],
            &original_devnet_setup.backend,
            &tx_validator_with_no_fee,
            Felt::ZERO,
        )
        .await;

        assert!(!original_devnet_setup.mempool.is_empty().await);

        // Start block production task with no_charge_fee = true.
        // This will execute the transaction and add it to the pre-confirmed block.
        let mut block_production_task = BlockProductionTask::new(
            original_devnet_setup.backend.clone(),
            original_devnet_setup.mempool.clone(),
            original_devnet_setup.metrics.clone(),
            Arc::new(original_devnet_setup.l1_client.clone()),
            false, // mempool_paused
            initial_no_charge_fee,
        );

        let mut notifications = block_production_task.subscribe_state_notifications();
        let restart_task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // Wait for transaction to be executed and added to pre-confirmed block
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Verify pre-confirmed block exists with our transaction
        assert!(original_devnet_setup.backend.has_preconfirmed_block());
        let preconfirmed_view = original_devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
        assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

        // Stop the task before it closes the block.
        // This simulates a node crash/restart scenario where a pre-confirmed block exists.
        drop(restart_task);
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Phase 2: Restart with different no_charge_fee value
        // This simulates a configuration change between shutdown and restart.
        let restart_no_charge_fee = false;
        let restart_block_production_task = BlockProductionTask::new(
            original_devnet_setup.backend.clone(), // Same backend = same database
            original_devnet_setup.mempool.clone(),
            original_devnet_setup.metrics.clone(),
            Arc::new(original_devnet_setup.l1_client.clone()),
            false,                 // mempool_paused
            restart_no_charge_fee, // Current config: no_charge_fee = false
        );

        // Start the block production task.
        // This will call setup_initial_state() which calls close_preconfirmed_block_if_exists().
        // During re-execution, it will use saved_no_charge_fee = true (from saved config),
        // NOT restart_no_charge_fee = false (from current config).
        let _restart_task = AbortOnDrop::spawn(async move {
            restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
        });

        // Give time for setup_initial_state to complete and close the pre-confirmed block
        tokio::time::sleep(Duration::from_secs(1)).await;

        // Phase 3: Verify block was closed successfully
        assert!(!original_devnet_setup.backend.has_preconfirmed_block());
        assert_eq!(original_devnet_setup.backend.latest_confirmed_block_n(), Some(1));

        // Phase 4: Verify config was updated with CURRENT value after re-execution
        // After re-execution completes, the config is updated to the current value.
        // This ensures that the next block will use the current configuration.
        let updated_config = original_devnet_setup
            .backend
            .get_runtime_exec_config()
            .expect("Should be able to read runtime exec config")
            .expect("Runtime exec config should exist after closing");

        assert_eq!(
            updated_config.no_charge_fee, restart_no_charge_fee,
            "Config should be updated with current value after re-execution completes"
        );
    }

    // This test verifies that graceful shutdown properly closes any open preconfirmed block
    // without requiring re-execution. When shutdown is triggered, the block production service
    // should close the preconfirmed block using the executor's existing state.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    async fn test_graceful_shutdown_closes_preconfirmed_block(
        #[future]
        #[with(Duration::from_secs(100), false)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        // Step 1: Set up block production with transactions
        assert!(devnet_setup.mempool.is_empty().await);

        // Add a transaction to the mempool
        sign_and_add_invoke_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.contracts.0[1],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;

        assert!(!devnet_setup.mempool.is_empty().await);

        // Step 2: Start block production and execute a batch to create a preconfirmed block
        let mut block_production_task = devnet_setup.block_prod_task();
        let mut notifications = block_production_task.subscribe_state_notifications();
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();

        let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

        // Wait for batch to be executed (transactions added to preconfirmed block)
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Verify preconfirmed block exists with transactions
        assert!(devnet_setup.backend.has_preconfirmed_block());
        let preconfirmed_view = devnet_setup.backend.block_view_on_current_preconfirmed().unwrap();
        assert_eq!(preconfirmed_view.num_executed_transactions(), 1);

        // Step 3: Trigger graceful shutdown by cancelling ServiceContext
        ctx_clone.cancel_global();

        // Step 4: Wait for EndFinalBlock to be processed (indicated by ClosedBlock notification)
        // During graceful shutdown:
        // - Batcher detects cancellation and exits, closing the send_batch channel
        // - Executor detects channel closure and sends EndFinalBlock message
        // - Main loop processes EndFinalBlock and closes the block (sends ClosedBlock notification)
        assert!(
            matches!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock { block_n: 1 }),
            "Expected ClosedBlock notification after EndFinalBlock was processed during graceful shutdown"
        );

        // Step 5: Wait for shutdown to complete
        // All database writes and head projection updates complete synchronously within the awaited rayon task,
        // so by the time task.await completes, the state is already updated. No delay needed.
        task.await.unwrap();

        // Step 6: Verify the preconfirmed block is closed and saved to database
        assert!(!devnet_setup.backend.has_preconfirmed_block(), "Preconfirmed block should be closed");

        // Verify block was properly closed (check latest confirmed block number)
        let latest_block_n = devnet_setup.backend.latest_confirmed_block_n();
        assert!(latest_block_n.is_some(), "Block should be closed and saved");
        let block_number = latest_block_n.unwrap();

        // Verify transactions are preserved correctly
        let closed_block = devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
        let executed_transactions = closed_block.get_executed_transactions(..).unwrap();
        assert_eq!(executed_transactions.len(), 1, "Transaction should be preserved in closed block");

        // Verify mempool is empty (transaction was consumed)
        assert!(devnet_setup.mempool.is_empty().await);
    }

    // This test verifies that graceful shutdown completes successfully when there is no
    // preconfirmed block to close. The shutdown should complete without errors.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    async fn test_graceful_shutdown_with_no_preconfirmed_block(
        #[future]
        #[with(Duration::from_secs(100), false)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        // Step 1: Start block production without adding any transactions
        // This ensures no preconfirmed block is created
        assert!(devnet_setup.mempool.is_empty().await);
        assert!(!devnet_setup.backend.has_preconfirmed_block());

        let block_production_task = devnet_setup.block_prod_task();
        let ctx = ServiceContext::new_for_testing();
        let ctx_clone = ctx.clone();

        let task = AbortOnDrop::spawn(async move { block_production_task.run(ctx).await });

        // Step 2: Give a small delay to ensure block production task is running
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Step 3: Verify no preconfirmed block exists
        assert!(!devnet_setup.backend.has_preconfirmed_block());

        // Step 4: Trigger graceful shutdown immediately
        ctx_clone.cancel_global();

        // Step 5: Wait for shutdown to complete - should complete without errors
        // Since there's no preconfirmed block, shutdown should complete immediately
        // without waiting for EndBlock
        task.await.unwrap();

        // Step 6: Verify shutdown completed successfully
        // No preconfirmed block should exist (still)
        assert!(!devnet_setup.backend.has_preconfirmed_block());
    }
}
