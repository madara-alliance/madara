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
mod close_pipeline;
mod close_queue;
mod current_block;
mod executor;
mod finalizer;
mod handle;
pub mod metrics;
mod recovery;
mod task;
mod util;

pub use handle::BlockProductionHandle;

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

/// Little state machine that helps us following the state transitions the executor thread sends us.
#[allow(clippy::large_enum_variant)]
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
    discard_preconfirmed_on_startup: bool,
    replay_mode_enabled: bool,
    parallel_merkle_enabled: bool,
    parallel_merkle_compare_sequential: bool,
    parallel_merkle_root_workers: usize,
    parallel_merkle_flush_interval: u64,
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
    /// * `discard_preconfirmed_on_startup`: Drops any recovered preconfirmed block instead of
    ///   re-executing and closing it during sequencer startup.
    ///
    /// # TODO(mohit 18/11/2025): Update the code to use config same as pre-close
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        mempool_paused: bool,
        no_charge_fee: bool,
        discard_preconfirmed_on_startup: bool,
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
            discard_preconfirmed_on_startup,
            replay_mode_enabled: false,
            parallel_merkle_enabled: false,
            parallel_merkle_compare_sequential: false,
            parallel_merkle_root_workers: 1,
            parallel_merkle_flush_interval: 3,
            diffs_since_snapshot: Vec::new(),
            pending_completions: VecDeque::new(),
        }
    }

    /// Sets the bounded number of blocks that may wait for or occupy finalization.
    pub fn with_close_queue_capacity(mut self, close_queue_capacity: usize) -> Self {
        self.close_queue_capacity = close_queue_capacity.max(1);
        self
    }

    /// Selects parallel root preparation with ordered commit when enabled.
    pub fn with_parallel_merkle_enabled(mut self, enabled: bool) -> Self {
        self.parallel_merkle_enabled = enabled;
        self
    }

    /// Enables root comparison against the sequential implementation for validation.
    pub fn with_parallel_merkle_compare_sequential(mut self, enabled: bool) -> Self {
        self.parallel_merkle_compare_sequential = enabled;
        self
    }

    /// Sets the maximum number of root computations allowed to run concurrently.
    pub fn with_parallel_merkle_root_workers(mut self, worker_count: u64) -> Self {
        self.parallel_merkle_root_workers = usize::try_from(worker_count).unwrap_or(usize::MAX).max(1);
        self
    }

    /// Enables runtime-only replay boundary behavior in the executor and handle.
    pub fn with_replay_mode_enabled(mut self, enabled: bool) -> Self {
        self.replay_mode_enabled = enabled;
        self.handle.set_replay_mode_enabled(enabled);
        self
    }

    /// Sets how many blocks are accumulated between durable trie boundaries.
    pub fn with_parallel_merkle_flush_interval(mut self, flush_interval: u64) -> Self {
        self.parallel_merkle_flush_interval = flush_interval.max(1);
        self
    }

    /// Returns a cloneable control handle for transaction submission and forced close.
    pub fn handle(&self) -> BlockProductionHandle {
        self.handle.clone()
    }

    /// This is a channel that helps the testing of the block production task. It is unused outside of tests.
    pub fn subscribe_state_notifications(&mut self) -> mpsc::UnboundedReceiver<BlockProductionStateNotification> {
        let (sender, recv) = mpsc::unbounded_channel();
        self.state_notifications = Some(sender);
        recv
    }

    /// Publishes a best-effort state transition to the optional test observer.
    fn send_state_notification(&mut self, notification: BlockProductionStateNotification) {
        if let Some(sender) = self.state_notifications.as_mut() {
            let _ = sender.send(notification);
        }
    }

    /// Records how many blocks currently occupy each in-memory pipeline stage.
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

    /// Returns a non-zero effective close-queue capacity.
    fn close_queue_capacity(&self) -> usize {
        self.close_queue_capacity.max(1)
    }

    /// Returns true when this block ends the configured durable Merkle interval.
    fn is_boundary_block(&self, block_n: u64) -> bool {
        let Some(next_block_n) = block_n.checked_add(1) else {
            return false;
        };
        self.parallel_merkle_flush_interval != 0
            && next_block_n.checked_rem(self.parallel_merkle_flush_interval) == Some(0)
    }
}

#[cfg(test)]
#[path = "tests.rs"]
pub(crate) mod tests;
