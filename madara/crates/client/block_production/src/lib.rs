//! Sequencer transaction execution and ordered block finalization. Full-node import uses `mc-sync`.
//!
//! # Ownership and data flow
//!
//! ```text
//! mempool / L1 messages / admin bypass
//!                  |
//!               Batcher
//!                  | bounded transaction batches
//!          dedicated executor thread
//!                  | StartNewBlock / BatchExecuted / EndBlock
//!          BlockProductionTask
//!                  | owned close payloads, bounded queue
//!              finalizer
//!                  | root computation -> ordered commit -> completion
//!          confirmed head and subscribers
//! ```
//!
//! The executor owns Blockifier state and can start the next block while previous blocks await
//! roots. The main task applies replies to block-scoped preconfirmed state, persisting executed
//! transactions when configured. An `EndBlock` transfers the completed block's state and summary
//! to the finalizer; it does not mean that the block is confirmed yet.
//!
//! # Three head positions
//!
//! The backend's `ChainHeadState` separates the confirmed tip, the external preconfirmed block
//! immediately after it, and the internal execution tip. For example, with block 100 confirmed
//! and execution at 103, public preconfirmed reads expose 101 while internal consumers track
//! 101 through 103. Confirming 101 advances the public projection to 102 and retains 102–103.
//! Mempool consumers must distinguish that confirmation from replacement of executed work.
//!
//! # Serial and parallel Merkle modes
//!
//! Both modes use the finalizer and publish confirmations in block order. Serial mode computes
//! and commits each root before accepting its completion. Parallel mode runs bounded root jobs
//! on Tokio's blocking pool, with Rayon used within root computation. Each job reads a pinned
//! checkpoint snapshot plus cumulative state diffs up to its block. Jobs may finish out of order;
//! the finalizer commits them in queue order.
//!
//! At a parallel checkpoint boundary the commit stage writes block parts, applies the root
//! job's cumulative trie overlay, persists and flushes checkpoint metadata, and only then
//! advances the confirmed head. If another boundary has made a job's base stale, its overlay is
//! skipped; its block can still be confirmed, and a later job catches the checkpoint up.
//! Between boundaries, block parts and their computed roots are persisted while the materialized
//! tries stay at the checkpoint. Root computation alone never publishes a head. Queue capacity
//! and root-worker limits provide separate backpressure.
//!
//! `--parallel-merkle-compare-sequential` runs an independent root calculation as a correctness
//! check; it adds computation and is intended for validation rather than normal operation.
//!
//! # Recovery and shutdown
//!
//! Producer startup reconciles materialized tries with the confirmed head before any preconfirmed
//! recovery, even when restarting in serial mode after parallel production. This includes serial
//! devnet genesis. The persisted confirmed head remains authoritative:
//! any trie state ahead of it is rolled back, and missing trie updates are rebuilt and verified
//! against its root. Persisted preconfirmed blocks are then re-executed in order using the saved
//! execution configuration. Explicit discard removes the suffix instead. Candidates are not
//! persisted and cannot be recovered from these records. Parallel startup checkpoints the recovered
//! head again before scheduling new roots.
//!
//! On normal shutdown the batcher closes the input channel, the executor sends `EndFinalBlock`,
//! and the main task drains and joins the finalizer. Parallel mode then reconciles the final
//! confirmed head to a checkpoint. The node registers block production with graceful shutdown
//! semantics so the service runner cannot report it stopped after merely exhausting a timeout.
//! This join is required before administrative database reverts. Errors propagate after cleanup;
//! unwinding aborts the async finalizer through its owning task handle.
//!
//! Replay mode separately constrains dispatch and execution to registered source-block
//! boundaries. Mempool pause affects only mempool intake; L1 messages and admin bypass remain
//! active. Replay boundaries and pause are runtime controls, not durable recovery records.

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
