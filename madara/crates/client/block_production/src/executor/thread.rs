//! Executor thread internal logic.

#[path = "thread_replay.rs"]
mod thread_replay;

use crate::metrics::BlockProductionMetrics;
use crate::util::{create_execution_context, BatchToExecute, BlockExecutionContext, ExecutionStats};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::{
    TransactionExecutionOutput, TransactionExecutor, TransactionExecutorResult,
};
use mc_db::MadaraBackend;
use mc_exec::metrics::{context_label, metrics as exec_metrics, tx_type_to_label};
use mc_exec::{execution::TxInfo, LayeredStateAdapter};
use mp_convert::{Felt, ToFelt};
use opentelemetry::KeyValue;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::ClassHash;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
    time::{Instant as StdInstant, SystemTime},
};
use tokio::{sync::mpsc, time::Instant};

struct ExecutorStateExecuting {
    exec_ctx: BlockExecutionContext,
    /// Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    /// bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    /// we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<LayeredStateAdapter>,
    declared_classes: HashMap<ClassHash, ContractClass>,
    consumed_l1_to_l2_nonces: HashSet<u64>,
    last_batch_finished_at: Option<StdInstant>,
}

struct ExecutorStateNewBlock {
    /// Keep the cached adaptor around to keep the cache around.
    state_adaptor: LayeredStateAdapter,
    consumed_l1_to_l2_nonces: HashSet<u64>,
    /// Wall-clock time captured when the previous block closed.
    /// Used as the next block's timestamp so that lazy execution context
    /// creation does not skew the timestamp forward.
    block_start_time: SystemTime,
}

/// Note: The reason this exists is because we want to create the new block execution context (meaning, the block header) as late as possible, as to have
/// the best gas prices. This is especially important when the no_empty_block configuration is enabled, as otherwise we would end up:
/// - Creating a new execution context, using the current gas prices.
/// - Waiting for a transaction to arrive.... potentially for a very, very long time..
/// - Transaction arrives, we execute it and close the block, as the block_time is reached.
///
/// At that point, the gas prices would be all wrong! In order to support no_empty_block correctly, we have to delay execution context creation
/// until the first transaction has arrived.
#[allow(clippy::large_enum_variant)]
enum ExecutorThreadState {
    /// A block has been started.
    Executing(ExecutorStateExecuting),
    /// Intermediate state, we do not have initialized the execution yet.
    NewBlock(ExecutorStateNewBlock),
}

impl ExecutorThreadState {
    /// Returns the L1 nonce set owned by the current executor phase.
    fn consumed_l1_to_l2_nonces(&mut self) -> &mut HashSet<u64> {
        match self {
            ExecutorThreadState::Executing(s) => &mut s.consumed_l1_to_l2_nonces,
            ExecutorThreadState::NewBlock(s) => &mut s.consumed_l1_to_l2_nonces,
        }
    }
    /// Returns a mutable reference to the state adapter.
    fn layered_state_adapter_mut(&mut self) -> &mut LayeredStateAdapter {
        match self {
            ExecutorThreadState::Executing(s) => {
                &mut s.executor.block_state.as_mut().expect("State already taken").state
            }
            ExecutorThreadState::NewBlock(s) => &mut s.state_adaptor,
        }
    }
}

/// Executor runs on a separate thread, as to avoid having tx popping, block closing etc. take precious time away that could
/// be spent executing the next tick instead.
/// This thread becomes the blockifier executor scheduler thread (via TransactionExecutor), which will internally spawn worker threads.
pub struct ExecutorThread {
    backend: Arc<MadaraBackend>,
    metrics: Arc<BlockProductionMetrics>,
    replay_mode_enabled: bool,

    incoming_batches: mpsc::Receiver<super::BatchToExecute>,
    replies_sender: mpsc::Sender<super::ExecutorMessage>,
    commands: mpsc::UnboundedReceiver<super::ExecutorCommand>,

    /// See `take_tx_batch`. When the mempool is empty, we will not be getting transactions.
    /// We still potentially want to emit empty blocks based on the block_time deadline.
    wait_rt: tokio::runtime::Runtime,
}

enum WaitTxBatchOutcome {
    /// Batch channel closed.
    Exit,
    /// The block deadline elapsed without new work.
    Deadline,
    /// Got a command to execute.
    Command(super::ExecutorCommand),
    /// Batch
    Batch(BatchToExecute),
}

impl WaitTxBatchOutcome {
    /// Returns the stable metric label for the event that ended the wait.
    fn metric_label(&self) -> &'static str {
        match self {
            Self::Exit => "closed",
            Self::Deadline => "timeout",
            Self::Command(_) => "command",
            Self::Batch(_) => "batch",
        }
    }

    /// Emits the detailed debug event that explains why the executor resumed.
    fn log_debug(&self) {
        match self {
            Self::Exit => tracing::debug!("Batch channel closed."),
            Self::Deadline => tracing::debug!("Executor wait deadline reached."),
            Self::Command(cmd) => tracing::debug!("Got cmd {cmd:?}."),
            Self::Batch(batch) => tracing::debug!("Got new batch with {} transactions.", batch.len()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CloseReason {
    ForceClose,
    ReplayBoundaryMet,
    BlockFull,
    BlockTimeDeadline,
}

impl CloseReason {
    /// Returns the low-cardinality metric label for one close trigger.
    fn as_label(self) -> &'static str {
        match self {
            CloseReason::ForceClose => "force_close",
            CloseReason::ReplayBoundaryMet => "replay_boundary_met",
            CloseReason::BlockFull => "block_full",
            CloseReason::BlockTimeDeadline => "block_time_deadline",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CloseDecision {
    pub(super) should_close: bool,
    pub(super) replay_boundary_exists: bool,
    pub(super) replay_boundary_met: bool,
    pub(super) reason: Option<CloseReason>,
}

/// Describes whether a close check kept the block open, closed it, or lost the reply channel.
///
/// Keeping channel shutdown explicit prevents it from being confused with a successful block transition.
enum CloseBlockOutcome {
    Open,
    Closed(ExecutorThreadState),
    Exit,
}

impl ExecutorThread {
    /// Creates the executor state machine and its dedicated wait runtime.
    pub fn new(
        backend: Arc<MadaraBackend>,
        incoming_batches: mpsc::Receiver<super::BatchToExecute>,
        replies_sender: mpsc::Sender<super::ExecutorMessage>,
        commands: mpsc::UnboundedReceiver<super::ExecutorCommand>,
        metrics: Arc<BlockProductionMetrics>,
        replay_mode_enabled: bool,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            backend,
            metrics,
            replay_mode_enabled,
            incoming_batches,
            replies_sender,
            commands,
            wait_rt: tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .context("Building tokio runtime")?,
        })
    }
    /// Records how long the executor waited and what resumed it.
    fn record_wait_for_work(&self, waited_secs: f64, outcome: &'static str, mode: &'static str) {
        self.metrics
            .executor_wait_for_work_duration
            .record(waited_secs, &[KeyValue::new("outcome", outcome), KeyValue::new("mode", mode)]);
    }

    /// Increments the counter for the selected block-close reason.
    fn record_close_reason(&self, reason: CloseReason) {
        self.metrics.executor_close_reason_total.add(1, &[KeyValue::new("reason", reason.as_label())]);
    }

    /// Returns [`WaitTxBatchOutcome::Exit`] when the input channel is closed.
    /// When `deadline` is `None`, this waits indefinitely for a batch or command.
    fn wait_take_tx_batch(&mut self, deadline: Option<Instant>, should_wait: bool) -> WaitTxBatchOutcome {
        if let Ok(batch) = self.incoming_batches.try_recv() {
            self.record_wait_for_work(0.0, "batch", "try_recv");
            let outcome = WaitTxBatchOutcome::Batch(batch);
            outcome.log_debug();
            return outcome;
        }

        if let Ok(cmd) = self.commands.try_recv() {
            self.record_wait_for_work(0.0, "command", "try_recv");
            let outcome = WaitTxBatchOutcome::Command(cmd);
            outcome.log_debug();
            return outcome;
        }

        if !should_wait {
            return WaitTxBatchOutcome::Batch(Default::default());
        }

        let wait_started = StdInstant::now();

        // tokio exposes blocking_recv but not a blocking recv-with-deadline helper, so this runtime-backed
        // select keeps the executor on a blocking thread while still honoring the block deadline when present.
        let result = self.wait_rt.block_on(async {
            match deadline {
                Some(deadline) => {
                    tokio::select! {
                        Some(cmd) = self.commands.recv() => {
                            WaitTxBatchOutcome::Command(cmd)
                        }
                        _ = tokio::time::sleep_until(deadline) => {
                            WaitTxBatchOutcome::Deadline
                        }
                        el = self.incoming_batches.recv() => match el {
                            Some(el) => WaitTxBatchOutcome::Batch(el),
                            None => WaitTxBatchOutcome::Exit,
                        }
                    }
                }
                None => {
                    tokio::select! {
                        Some(cmd) = self.commands.recv() => {
                            WaitTxBatchOutcome::Command(cmd)
                        }
                        el = self.incoming_batches.recv() => match el {
                            Some(el) => WaitTxBatchOutcome::Batch(el),
                            None => WaitTxBatchOutcome::Exit,
                        }
                    }
                }
            }
        });
        self.record_wait_for_work(wait_started.elapsed().as_secs_f64(), result.metric_label(), "recv");
        result.log_debug();
        result
    }

    /// We are making a new block - we need to put the hash of current_block_n-10 into the state diff.
    /// current_block_n-10 however might not be saved into the database yet. In that case, we have to wait.
    /// This shouldn't create a deadlock (cyclic wait) unless the database is in a weird state (?)
    ///
    /// https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1
    fn wait_for_hash_of_block_min_10(&self, block_n: u64) -> anyhow::Result<Option<(u64, Felt)>> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else { return Ok(None) };

        let get_hash_from_db = || {
            if let Some(view) = self.backend.block_view_on_confirmed(block_n_min_10) {
                // block exists
                anyhow::Ok(Some(view.get_block_info().context("Getting block hash of block_n - 10")?.block_hash))
            } else {
                Ok(None)
            }
        };

        // Optimistically get the hash from database without subscribing to the closed_blocks channel.
        if let Some(block_hash) = get_hash_from_db()? {
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            tracing::debug!(
                "executor_waiting_for_confirmed_hash required_block={} current_block={}",
                block_n_min_10,
                block_n
            );
            let wait_started = std::time::Instant::now();
            loop {
                let mut receiver = self.backend.watch_chain_head_state();
                // We need to re-query the DB here since the it is possible for the block hash to have arrived just in between.
                if let Some(block_hash) = get_hash_from_db()? {
                    tracing::debug!(
                        "executor_confirmed_hash_available required_block={} current_block={} wait_ms={}",
                        block_n_min_10,
                        block_n,
                        wait_started.elapsed().as_secs_f64() * 1000.0
                    );
                    break Ok(Some((block_n_min_10, block_hash)));
                }
                tracing::debug!(
                    "executor_confirmed_hash_still_pending required_block={} current_block={}",
                    block_n_min_10,
                    block_n
                );
                self.wait_rt.block_on(async { receiver.recv().await });
            }
        }
    }

    /// End the current block.
    fn end_block(&mut self, state: &mut ExecutorStateExecuting) -> anyhow::Result<ExecutorThreadState> {
        let mut cached_state = state.executor.block_state.take().expect("Executor block state already taken");

        let state_diff = cached_state.to_state_diff().context("Cannot make state diff")?.state_maps;
        let mut cached_adapter = cached_state.state;
        cached_adapter.finish_block(
            state_diff,
            mem::take(&mut state.declared_classes),
            mem::take(&mut state.consumed_l1_to_l2_nonces),
        )?;

        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: cached_adapter,
            consumed_l1_to_l2_nonces: HashSet::new(),
            block_start_time: SystemTime::now(),
        }))
    }

    /// Returns the initial state diff storage too. It is used to create the StartNewBlock message and transition to ExecutorState::Executing.
    fn create_execution_state(
        &mut self,
        state: ExecutorStateNewBlock,
        previous_l2_gas_used: u128,
    ) -> anyhow::Result<ExecutorStateExecuting> {
        let previous_l2_gas_price = state.state_adaptor.latest_gas_prices().strk_l2_gas_price;
        // When no_empty_blocks is enabled, blocks are produced on-demand and the
        // wait for the first tx can be arbitrarily long. Use wall-clock time so
        // the timestamp reflects when the block actually started executing.
        // Otherwise use the time captured when the previous block closed, so that
        // consecutive blocks have timestamps spaced by ~block_time.
        let block_timestamp =
            if self.backend.chain_config().no_empty_blocks { SystemTime::now() } else { state.block_start_time };
        let exec_ctx = create_execution_context(
            &self.backend,
            state.state_adaptor.block_n(),
            previous_l2_gas_price,
            previous_l2_gas_used,
            block_timestamp,
        )?;

        // Create the TransactionExecutor with block_n-10 handling, reusing the layered_state_adapter.
        let executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state.state_adaptor,
            |block_n| self.wait_for_hash_of_block_min_10(block_n),
            None, // Use backend's chain_config (normal execution)
        )?;

        Ok(ExecutorStateExecuting {
            exec_ctx,
            executor,
            consumed_l1_to_l2_nonces: state.consumed_l1_to_l2_nonces,
            declared_classes: HashMap::new(),
            last_batch_finished_at: None,
        })
    }

    /// Reconstructs the first executor phase from the backend's confirmed head.
    fn initial_state(&self) -> anyhow::Result<ExecutorThreadState> {
        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: LayeredStateAdapter::new(Arc::clone(&self.backend))?,
            consumed_l1_to_l2_nonces: HashSet::new(),
            block_start_time: SystemTime::now(),
        }))
    }

    /// Finalizes any executing block when the batch channel closes.
    ///
    /// A failed or undeliverable finalization leaves persisted preconfirmed data for startup recovery.
    fn finish_shutdown(&mut self, state: &mut ExecutorThreadState) {
        let ExecutorThreadState::Executing(execution_state) = state else {
            tracing::debug!("Shutting down executor, no block to close");
            if self.replies_sender.blocking_send(super::ExecutorMessage::EndFinalBlock(None)).is_err() {
                tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
            }
            return;
        };

        tracing::debug!("Shutting down executor, closing block block_n={}", execution_state.exec_ctx.block_number);
        let started_at = Instant::now();
        match execution_state.executor.finalize() {
            Ok(summary) => {
                let elapsed = started_at.elapsed().as_secs_f64();
                self.metrics.executor_finalize_duration.record(elapsed, &[]);
                self.metrics.executor_finalize_last.record(elapsed, &[]);
                if self
                    .replies_sender
                    .blocking_send(super::ExecutorMessage::EndFinalBlock(Some(Box::new(summary))))
                    .is_err()
                {
                    tracing::warn!("Could not send EndFinalBlock during shutdown, block will remain preconfirmed");
                }
            }
            Err(error) => {
                if self.replies_sender.blocking_send(super::ExecutorMessage::EndFinalBlock(None)).is_err() {
                    tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
                }
                tracing::warn!("Failed to finalize block during shutdown: {:?}. Block will remain preconfirmed", error);
            }
        }
    }

    /// Converts a wait result into executable work or a graceful loop termination.
    ///
    /// Force-close acknowledgements are sent immediately; deadlines become empty batches.
    fn resolve_wait_outcome(
        &mut self,
        outcome: WaitTxBatchOutcome,
        state: &mut ExecutorThreadState,
        force_close: &mut bool,
    ) -> Option<BatchToExecute> {
        match outcome {
            WaitTxBatchOutcome::Batch(batch) => Some(batch),
            WaitTxBatchOutcome::Deadline => Some(BatchToExecute::default()),
            WaitTxBatchOutcome::Command(super::ExecutorCommand::CloseBlock(callback)) => {
                *force_close = true;
                let _ = callback.send(Ok(()));
                Some(BatchToExecute::default())
            }
            WaitTxBatchOutcome::Exit => {
                self.finish_shutdown(state);
                None
            }
        }
    }

    /// Appends newly received work while filtering duplicate consumed L1-handler nonces.
    ///
    /// The nonce is reserved in the current block before the transaction enters execution.
    fn append_received_transactions(
        state: &mut ExecutorThreadState,
        to_exec: &mut BatchToExecute,
        taken: BatchToExecute,
    ) -> anyhow::Result<()> {
        for (tx, additional_info) in taken {
            if let Some(nonce) = tx.l1_handler_tx_nonce() {
                let nonce: u64 = nonce.to_felt().try_into().context("Converting nonce from felt to u64")?;
                if state
                    .layered_state_adapter_mut()
                    .is_l1_to_l2_message_nonce_consumed(nonce)
                    .context("Checking is l1 to l2 message nonce is already consumed")?
                    || !state.consumed_l1_to_l2_nonces().insert(nonce)
                {
                    tracing::debug!("L1 Core Contract nonce already consumed: {nonce}");
                    continue;
                }
            }
            to_exec.push(tx, additional_info);
        }
        Ok(())
    }

    /// Creates an executing block state and publishes its execution context to the main task.
    ///
    /// A closed reply channel returns `None`, signaling the executor loop to stop.
    fn start_executing_block(
        &mut self,
        state: ExecutorStateNewBlock,
        previous_l2_gas_used: u128,
    ) -> anyhow::Result<Option<ExecutorStateExecuting>> {
        let execution_state =
            self.create_execution_state(state, previous_l2_gas_used).context("Creating execution state")?;
        tracing::debug!("Starting new block, block_n={}", execution_state.exec_ctx.block_number);
        let message = super::ExecutorMessage::StartNewBlock { exec_ctx: execution_state.exec_ctx.clone() };
        Ok(self.replies_sender.blocking_send(message).is_ok().then_some(execution_state))
    }

    /// Summarizes execution results and updates per-block declared-class and gas state.
    ///
    /// Result conversion remains asynchronous; this pass records only scheduler statistics.
    fn summarize_execution_results(
        execution_state: &mut ExecutorStateExecuting,
        executed_txs: &BatchToExecute,
        results: &[TransactionExecutorResult<TransactionExecutionOutput>],
        exec_duration: std::time::Duration,
        block_empty: &mut bool,
    ) -> (ExecutionStats, Vec<Felt>) {
        let mut stats =
            ExecutionStats { n_batches: 1, n_executed: executed_txs.len(), exec_duration, ..Default::default() };
        let avg_tx_time_ms =
            if results.is_empty() { 0.0 } else { exec_duration.as_secs_f64() * 1000.0 / results.len() as f64 };
        let mut replay_hashes = Vec::new();
        for (transaction, result) in executed_txs.txs.iter().zip(results) {
            match result {
                Ok((execution_info, _)) => {
                    tracing::trace!("Successful execution of transaction {:#x}", transaction.tx_hash().to_felt());
                    exec_metrics().record_tx_execution_time(
                        avg_tx_time_ms,
                        tx_type_to_label(transaction.tx_type()),
                        context_label::PRODUCTION,
                    );
                    stats.n_added_to_block += 1;
                    replay_hashes.push(transaction.tx_hash().to_felt());
                    stats.l2_gas_consumed += u128::from(execution_info.receipt.gas.l2_gas.0);
                    *block_empty = false;
                    if execution_info.revert_error.is_some() {
                        stats.n_reverted += 1;
                    } else if let Some((class_hash, contract_class)) = transaction.declared_contract_class() {
                        tracing::debug!("Declared class_hash={:#x}", class_hash.to_felt());
                        stats.declared_classes += 1;
                        execution_state.declared_classes.insert(class_hash, contract_class);
                    }
                }
                Err(error) => {
                    tracing::error!(
                        "Rejected transaction {:#x} for unexpected error: {error:#}",
                        transaction.tx_hash().to_felt()
                    );
                    stats.n_rejected += 1;
                }
            }
        }
        (stats, replay_hashes)
    }

    /// Executes the buffered transactions and returns the main-task reply plus the bouncer-full flag.
    ///
    /// Transactions beyond the returned result count remain buffered for the next block.
    fn execute_batch(
        &self,
        execution_state: &mut ExecutorStateExecuting,
        to_exec: &mut BatchToExecute,
        block_empty: &mut bool,
    ) -> (super::BatchExecutionResult, bool, Vec<Felt>) {
        let started_at = Instant::now();
        if let Some(waited) =
            execution_state.last_batch_finished_at.map(|last_finished_at| last_finished_at.elapsed().as_secs_f64())
        {
            self.metrics.executor_inter_batch_wait_duration.record(waited, &[]);
        }
        let results = execution_state.executor.execute_txs(&to_exec.txs, /* execution_deadline */ None);
        let exec_duration = started_at.elapsed();
        let block_full = results.len() < to_exec.len();
        let executed_txs = to_exec.remove_n_front(results.len());
        let (stats, replay_hashes) =
            Self::summarize_execution_results(execution_state, &executed_txs, &results, exec_duration, block_empty);
        execution_state.last_batch_finished_at = Some(StdInstant::now());

        tracing::debug!("Finished batch execution.");
        tracing::debug!("Stats: {:?}", stats);
        tracing::debug!(
            "Weights: {:?}",
            execution_state.executor.bouncer.lock().expect("Bouncer lock poisoned").get_bouncer_weights()
        );
        tracing::debug!("Block now full: {:?}", block_full);
        if let Some(block_state) = execution_state.executor.block_state.as_mut() {
            block_state.state.evict_read_cache_if_needed();
        }
        (
            super::BatchExecutionResult {
                executed_txs,
                blockifier_results: results,
                stats,
                emitted_at: StdInstant::now(),
            },
            block_full,
            replay_hashes,
        )
    }

    /// Applies close policy, finalizes a matching block, and sends its summary.
    ///
    /// The returned state transition is applied by the loop after the current mutable borrow ends.
    fn close_block_if_ready(
        &mut self,
        execution_state: &mut ExecutorStateExecuting,
        force_close: bool,
        block_full: bool,
        deadline: Instant,
    ) -> anyhow::Result<CloseBlockOutcome> {
        let block_n = execution_state.exec_ctx.block_number;
        let decision = self.replay_close_decision(block_n, force_close, block_full, Instant::now() >= deadline);
        if !decision.should_close {
            return Ok(CloseBlockOutcome::Open);
        }
        if let Some(reason) = decision.reason {
            self.record_close_reason(reason);
        }

        tracing::debug!("Ending block block_n={block_n}");
        let started_at = Instant::now();
        let summary = execution_state.executor.finalize()?;
        let elapsed = started_at.elapsed().as_secs_f64();
        self.metrics.executor_finalize_duration.record(elapsed, &[]);
        self.metrics.executor_finalize_last.record(elapsed, &[]);
        if self.replies_sender.blocking_send(super::ExecutorMessage::EndBlock(Box::new(summary))).is_err() {
            return Ok(CloseBlockOutcome::Exit);
        }
        Ok(CloseBlockOutcome::Closed(self.end_block(execution_state).context("Ending block")?))
    }

    /// Drives the executor state machine until its batch or reply channel closes.
    ///
    /// Each iteration receives work, executes one capped batch, and evaluates the block-close policy.
    pub fn run(mut self) -> anyhow::Result<()> {
        let batch_size = self.backend.chain_config().block_production_concurrency.batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;
        let mut state = self.initial_state().context("Creating executor initial state")?;
        let mut to_exec = BatchToExecute::with_capacity(batch_size);
        let mut replay_next_block_buffer = BatchToExecute::with_capacity(batch_size);
        let mut next_block_deadline = Instant::now() + block_time;
        let mut force_close = false;
        let mut block_empty = true;
        let mut l2_gas_consumed_block = 0;
        tracing::debug!("Starting executor thread.");

        loop {
            if to_exec.len() < batch_size {
                let deadline = self.replay_wait_deadline(&state, block_empty, no_empty_blocks, next_block_deadline);
                let outcome = self.wait_take_tx_batch(deadline, /* should_wait */ to_exec.is_empty());
                let Some(taken) = self.resolve_wait_outcome(outcome, &mut state, &mut force_close) else {
                    return Ok(());
                };
                Self::append_received_transactions(&mut state, &mut to_exec, taken)?;
            }

            let execution_state = match state {
                ExecutorThreadState::Executing(ref mut executing) => executing,
                ExecutorThreadState::NewBlock(new_block) => {
                    let Some(executing) = self.start_executing_block(new_block, l2_gas_consumed_block)? else {
                        return Ok(());
                    };
                    l2_gas_consumed_block = 0;
                    state = ExecutorThreadState::Executing(executing);
                    let ExecutorThreadState::Executing(executing) = &mut state else { unreachable!() };
                    executing
                }
            };
            self.apply_replay_boundary_capacity(
                execution_state.exec_ctx.block_number,
                &mut to_exec,
                &mut replay_next_block_buffer,
            );
            let (exec_result, block_full, replay_hashes) =
                self.execute_batch(execution_state, &mut to_exec, &mut block_empty);
            l2_gas_consumed_block += exec_result.stats.l2_gas_consumed;
            self.record_replay_executed_hashes(execution_state.exec_ctx.block_number, &replay_hashes);
            if exec_result.stats.n_executed > 0
                && self.replies_sender.blocking_send(super::ExecutorMessage::BatchExecuted(exec_result)).is_err()
            {
                return Ok(());
            }

            match self.close_block_if_ready(execution_state, force_close, block_full, next_block_deadline)? {
                CloseBlockOutcome::Open => {}
                CloseBlockOutcome::Exit => return Ok(()),
                CloseBlockOutcome::Closed(next_state) => {
                    state = next_state;
                    next_block_deadline = Instant::now() + block_time;
                    block_empty = true;
                    force_close = false;
                    if !replay_next_block_buffer.is_empty() {
                        to_exec.extend(mem::take(&mut replay_next_block_buffer));
                    }
                }
            }
        }
    }
}
