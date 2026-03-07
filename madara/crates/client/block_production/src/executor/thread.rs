//! Executor thread internal logic.

use crate::metrics::BlockProductionMetrics;
use crate::util::{create_execution_context, BatchToExecute, BlockExecutionContext, ExecutionStats};
use anyhow::Context;
use blockifier::blockifier::transaction_executor::TransactionExecutor;
use futures::future::OptionFuture;
use mc_db::MadaraBackend;
use mc_exec::metrics::{context_label, metrics as exec_metrics, tx_type_to_label};
use mc_exec::{execution::TxInfo, LayeredStateAdapter};
use mp_convert::{Felt, ToFelt};
use starknet_api::contract_class::ContractClass;
use starknet_api::core::ClassHash;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
    time::Instant as StdInstant,
};
use tokio::{sync::mpsc, time::Instant};

const EXECUTOR_IDLE_INFO_MS: f64 = 100.0;

struct ExecutorStateExecuting {
    exec_ctx: BlockExecutionContext,
    /// Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    /// bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    /// we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<LayeredStateAdapter>,
    declared_classes: HashMap<ClassHash, ContractClass>,
    consumed_l1_to_l2_nonces: HashSet<u64>,
}

struct ExecutorStateNewBlock {
    /// Keep the cached adaptor around to keep the cache around.
    state_adaptor: LayeredStateAdapter,
    consumed_l1_to_l2_nonces: HashSet<u64>,
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
    /// Got a command to execute.
    Command(super::ExecutorCommand),
    /// Batch
    Batch(BatchToExecute),
}

impl ExecutorThread {
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
    /// Returns None when the channel is closed.
    /// We want to close down the thread in that case.
    fn wait_take_tx_batch(
        &mut self,
        current_block_n: Option<u64>,
        deadline: Option<Instant>,
        should_wait: bool,
    ) -> WaitTxBatchOutcome {
        if let Ok(batch) = self.incoming_batches.try_recv() {
            tracing::debug!(
                "executor_batch_received block_number={current_block_n:?} tx_count={} receive_mode=try_recv",
                batch.len()
            );
            return WaitTxBatchOutcome::Batch(batch);
        }

        if let Ok(cmd) = self.commands.try_recv() {
            tracing::debug!(
                "executor_command_received block_number={current_block_n:?} command={cmd:?} receive_mode=try_recv"
            );
            return WaitTxBatchOutcome::Command(cmd);
        }

        if !should_wait {
            return WaitTxBatchOutcome::Batch(Default::default());
        }

        let now = Instant::now();
        let deadline_remaining_ms =
            deadline.map(|deadline| deadline.saturating_duration_since(now).as_secs_f64() * 1000.0);
        if deadline_remaining_ms.is_none_or(|remaining| remaining > 1.0) {
            tracing::debug!(
                "executor_waiting_for_batch block_number={current_block_n:?} until_block_time_deadline={} deadline_remaining_ms={deadline_remaining_ms:?}",
                deadline.is_some()
            );
        }
        let wait_started = StdInstant::now();

        // nb: tokio has blocking_recv, but no blocking_recv_timeout? this kinda sucks :(
        // especially because they do have it implemented in send_timeout and internally, they just have not exposed the
        // function.
        // Should be fine, as we optimistically try_recv above and we should only hit this when we actually have to wait.
        // nb.2: use an async block here, as timeout_at needs a runtime to be available on creation.
        self.wait_rt.block_on(async {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                    tracing::debug!(
                        "executor_command_received block_number={current_block_n:?} command={cmd:?} wait_ms={wait_ms} receive_mode=recv"
                    );
                    WaitTxBatchOutcome::Command(cmd)
                }
                _ = OptionFuture::from(deadline.map(tokio::time::sleep_until)) => {
                    let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                    if wait_ms > 1.0 {
                        tracing::debug!(
                            "executor_batch_wait_timed_out block_number={current_block_n:?} wait_ms={wait_ms}"
                        );
                    }
                    WaitTxBatchOutcome::Batch(Default::default())
                }
                el = self.incoming_batches.recv() => match el {
                    Some(el) => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        if wait_ms >= EXECUTOR_IDLE_INFO_MS {
                            tracing::info!(
                                "executor_batch_received_after_wait block_number={current_block_n:?} tx_count={} wait_ms={wait_ms} receive_mode=recv",
                                el.len()
                            );
                        } else {
                            tracing::debug!(
                                "executor_batch_received block_number={current_block_n:?} tx_count={} wait_ms={wait_ms} receive_mode=recv",
                                el.len()
                            );
                        }
                        WaitTxBatchOutcome::Batch(el)
                    }
                    None => {
                        let wait_ms = wait_started.elapsed().as_secs_f64() * 1000.0;
                        tracing::info!(
                            "executor_batch_channel_closed block_number={current_block_n:?} wait_ms={wait_ms}"
                        );
                        WaitTxBatchOutcome::Exit
                    }
                }
            }
        })
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
            tracing::info!(
                "executor_waiting_for_confirmed_hash required_block={} current_block={}",
                block_n_min_10,
                block_n
            );
            let wait_started = std::time::Instant::now();
            loop {
                let mut receiver = self.backend.watch_chain_head_state();
                // We need to re-query the DB here since the it is possible for the block hash to have arrived just in between.
                if let Some(block_hash) = get_hash_from_db()? {
                    tracing::info!(
                        "executor_confirmed_hash_available required_block={} current_block={} wait_ms={}",
                        block_n_min_10,
                        block_n,
                        wait_started.elapsed().as_secs_f64() * 1000.0
                    );
                    break Ok(Some((block_n_min_10, block_hash)));
                }
                tracing::info!(
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
        }))
    }

    /// Returns the initial state diff storage too. It is used to create the StartNewBlock message and transition to ExecutorState::Executing.
    fn create_execution_state(
        &mut self,
        state: ExecutorStateNewBlock,
        previous_l2_gas_used: u128,
    ) -> anyhow::Result<ExecutorStateExecuting> {
        let previous_l2_gas_price = state.state_adaptor.latest_gas_prices().strk_l2_gas_price;
        let exec_ctx = create_execution_context(
            &self.backend,
            state.state_adaptor.block_n(),
            previous_l2_gas_price,
            previous_l2_gas_used,
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
        })
    }

    fn initial_state(&self) -> anyhow::Result<ExecutorThreadState> {
        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: LayeredStateAdapter::new(Arc::clone(&self.backend))?,
            consumed_l1_to_l2_nonces: HashSet::new(),
        }))
    }

    fn replay_boundary_exists(&self, block_n: u64) -> bool {
        self.backend.replay_boundary_exists(block_n)
    }

    fn replay_boundary_remaining_capacity(&self, block_n: u64) -> Option<usize> {
        self.backend
            .replay_boundary_remaining_execution_capacity(block_n)
            .map(|remaining| usize::try_from(remaining).unwrap_or(usize::MAX))
    }

    fn replay_boundary_is_met(&self, block_n: u64) -> Option<bool> {
        self.backend.replay_boundary_is_met(block_n)
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        let batch_size = self.backend.chain_config().block_production_concurrency.batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        // Initial state is ExecutorState::NewBlock, we don't yet have an execution state.
        let mut state = self.initial_state().context("Creating executor initial state")?;

        // The batch of transactions to execute.
        let mut to_exec = BatchToExecute::with_capacity(batch_size);
        let mut replay_next_block_buffer = BatchToExecute::with_capacity(batch_size);

        let mut next_block_deadline = Instant::now() + block_time;
        let mut force_close = false;
        let mut block_empty = true;
        let mut l2_gas_consumed_block = 0;

        tracing::debug!("Starting executor thread.");

        // The goal here is to do the least possible between batches, as to maximize CPU usage. Any millisecond spent
        //  outside `TransactionExecutor::execute_txs` is a millisecond where we could have used every CPU core, but are using only one.
        // `blockifier` isn't really well optimized in this regard, but since we can't easily change its code (maybe we should?), we're
        //  still optimizing everything we have a hand on here in madara.
        loop {
            // Take transactions to execute.
            if to_exec.len() < batch_size {
                let replay_boundary_active = self.replay_mode_enabled
                    && matches!(
                        &state,
                        ExecutorThreadState::Executing(executing_state)
                            if self.replay_boundary_exists(executing_state.exec_ctx.block_number)
                    );
                let wait_deadline = if replay_boundary_active || (block_empty && no_empty_blocks) {
                    None
                } else {
                    Some(next_block_deadline)
                };
                // should_wait: We don't want to wait if we already have transactions to process - but we would still like to fill up our batch if possible.

                let current_block_n = match &state {
                    ExecutorThreadState::Executing(executing_state) => Some(executing_state.exec_ctx.block_number),
                    ExecutorThreadState::NewBlock(new_block_state) => Some(new_block_state.state_adaptor.block_n()),
                };
                let taken = match self.wait_take_tx_batch(
                    current_block_n,
                    wait_deadline,
                    /* should_wait */ to_exec.is_empty(),
                ) {
                    // Got a batch
                    WaitTxBatchOutcome::Batch(batch_to_execute) => batch_to_execute,
                    // Got a command
                    WaitTxBatchOutcome::Command(executor_command) => match executor_command {
                        super::ExecutorCommand::CloseBlock(callback) => {
                            force_close = true;
                            let _ = callback.send(Ok(()));
                            Default::default()
                        }
                    },
                    // Channel closed. Exit gracefully.
                    // Before exiting, check if we have an executing block that needs to be closed.
                    // This ensures graceful shutdown closes the block using the executor's existing state.
                    WaitTxBatchOutcome::Exit => {
                        match state {
                            ExecutorThreadState::Executing(mut execution_state) => {
                                tracing::debug!(
                                    "Shutting down executor, closing block block_n={}",
                                    execution_state.exec_ctx.block_number
                                );

                                // Finalize the block to get execution summary
                                // This uses the executor's current state - no re-execution needed
                                let finalize_start = Instant::now();
                                match execution_state.executor.finalize() {
                                    Ok(block_exec_summary) => {
                                        let finalize_secs = finalize_start.elapsed().as_secs_f64();
                                        self.metrics.executor_finalize_duration.record(finalize_secs, &[]);
                                        self.metrics.executor_finalize_last.record(finalize_secs, &[]);

                                        // Send EndFinalBlock message so main loop can close the block during shutdown
                                        if self
                                            .replies_sender
                                            .blocking_send(super::ExecutorMessage::EndFinalBlock(Some(Box::new(
                                                block_exec_summary,
                                            ))))
                                            .is_err()
                                        {
                                            // Receiver closed - main loop already shut down
                                            // Block will remain preconfirmed and be handled on restart
                                            tracing::warn!(
                                                "Could not send EndFinalBlock during shutdown, block will remain preconfirmed"
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        // Finalization failed - log error but continue shutdown
                                        // Block will remain preconfirmed and be handled on restart
                                        if self
                                            .replies_sender
                                            .blocking_send(super::ExecutorMessage::EndFinalBlock(None))
                                            .is_err()
                                        {
                                            tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
                                        }
                                        tracing::warn!(
                                            "Failed to finalize block during shutdown: {:?}. Block will remain preconfirmed",
                                            e
                                        );
                                    }
                                }
                            }
                            ExecutorThreadState::NewBlock(_) => {
                                // No block to close, send EndFinalBlock(None) to signal completion
                                tracing::debug!("Shutting down executor, no block to close");
                                if self
                                    .replies_sender
                                    .blocking_send(super::ExecutorMessage::EndFinalBlock(None))
                                    .is_err()
                                {
                                    // Receiver closed - main loop already shut down
                                    tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
                                }
                            }
                        }

                        return Ok(());
                    }
                };

                for (tx, additional_info) in taken {
                    // Remove duplicate l1handlertxs. We want to be absolutely sure we're not duplicating them.
                    if let Some(nonce) = tx.l1_handler_tx_nonce() {
                        let nonce: u64 = nonce.to_felt().try_into().context("Converting nonce from felt to u64")?;

                        if state
                            .layered_state_adapter_mut()
                            .is_l1_to_l2_message_nonce_consumed(nonce)
                            .context("Checking is l1 to l2 message nonce is already consumed")?
                            || !state.consumed_l1_to_l2_nonces().insert(nonce)
                        // insert: Returns true if it was already consumed in the current state.
                        {
                            tracing::debug!("L1 Core Contract nonce already consumed: {nonce}");
                            continue;
                        }
                    }
                    to_exec.push(tx, additional_info)
                }
            }

            // Create a new execution state (new block) if it does not already exist.
            // This transitions the state machine from ExecutorState::NewBlock to ExecutorState::Executing, and
            // creates the blockifier TransactionExecutor.
            let execution_state = match state {
                ExecutorThreadState::Executing(ref mut executor_state_executing) => executor_state_executing,
                ExecutorThreadState::NewBlock(state_new_block) => {
                    // Create new execution state.
                    let create_state_start = Instant::now();
                    let execution_state = self
                        .create_execution_state(state_new_block, l2_gas_consumed_block)
                        .context("Creating execution state")?;
                    l2_gas_consumed_block = 0;
                    tracing::debug!(
                        block_number = execution_state.exec_ctx.block_number,
                        create_execution_state_ms = create_state_start.elapsed().as_secs_f64() * 1000.0,
                        "executor_new_block_state_created"
                    );

                    tracing::info!("executor_start_new_block block_number={}", execution_state.exec_ctx.block_number);
                    if self
                        .replies_sender
                        .blocking_send(super::ExecutorMessage::StartNewBlock {
                            exec_ctx: execution_state.exec_ctx.clone(),
                        })
                        .is_err()
                    {
                        // Receiver closed
                        break Ok(());
                    }

                    // Replace the state with ExecutorState::Executing while returning a mutable reference to it.
                    // I wish rust had a better way to do that :/
                    state = ExecutorThreadState::Executing(execution_state);
                    let ExecutorThreadState::Executing(execution_state) = &mut state else { unreachable!() };
                    execution_state
                }
            };

            if self.replay_mode_enabled {
                let block_n = execution_state.exec_ctx.block_number;
                if let Some(remaining) = self.replay_boundary_remaining_capacity(block_n) {
                    if to_exec.len() > remaining {
                        let overflow_txs = to_exec.txs.split_off(remaining);
                        let overflow_additional_info = to_exec.additional_info.split_off(remaining);
                        let overflow_count = overflow_txs.len();
                        replay_next_block_buffer
                            .extend(BatchToExecute { txs: overflow_txs, additional_info: overflow_additional_info });
                        tracing::debug!(
                            "replay_boundary_executor_slice block_number={} remaining_for_block={} moved_to_next_block={}",
                            block_n,
                            remaining,
                            overflow_count
                        );
                    }
                }
            }

            let exec_start_time = Instant::now();
            let has_txs_in_batch = !to_exec.is_empty();
            let (first_tx_hash, first_tx_type) = if has_txs_in_batch {
                let first_tx = to_exec.txs.first().expect("non-empty batch must have a first tx");
                (format!("{:#x}", first_tx.tx_hash().to_felt()), tx_type_to_label(first_tx.tx_type()))
            } else {
                (String::new(), "")
            };
            if has_txs_in_batch {
                tracing::debug!(
                    "executor_batch_execution_started block_number={} txs_in_batch={} first_tx_hash={} first_tx_type={}",
                    execution_state.exec_ctx.block_number,
                    to_exec.len(),
                    first_tx_hash,
                    first_tx_type
                );
            }

            // TODO: we should use the execution deadline option
            // Execute the transactions.
            let blockifier_results =
                execution_state.executor.execute_txs(&to_exec.txs, /* execution_deadline */ None);

            let exec_duration = exec_start_time.elapsed();

            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            let block_full = blockifier_results.len() < to_exec.len();

            let executed_txs = to_exec.remove_n_front(blockifier_results.len()); // Remove the used txs.

            let mut stats = ExecutionStats::default();
            stats.n_batches += 1;
            stats.n_executed += executed_txs.len();
            stats.exec_duration += exec_duration;

            // Doesn't process the results, it just inspects them for logging stats, and figures out which classes were declared.
            // Results are processed async, outside the executor.
            // Calculate average per-tx time for metrics (batch executes all txs together).
            let avg_tx_time_ms = if !blockifier_results.is_empty() {
                exec_duration.as_secs_f64() * 1000.0 / blockifier_results.len() as f64
            } else {
                0.0
            };
            let mut replay_executed_hashes: Vec<Felt> = Vec::new();
            for (btx, res) in executed_txs.txs.iter().zip(blockifier_results.iter()) {
                match res {
                    Ok((execution_info, _state_diff)) => {
                        tracing::trace!("Successful execution of transaction {:#x}", btx.tx_hash().to_felt());

                        // Record tx execution time metric with production context.
                        exec_metrics().record_tx_execution_time(
                            avg_tx_time_ms,
                            tx_type_to_label(btx.tx_type()),
                            context_label::PRODUCTION,
                        );

                        stats.n_added_to_block += 1;
                        replay_executed_hashes.push(btx.tx_hash().to_felt());
                        stats.l2_gas_consumed += u128::from(execution_info.receipt.gas.l2_gas.0);
                        block_empty = false;
                        if execution_info.revert_error.is_some() {
                            stats.n_reverted += 1;
                        } else if let Some((class_hash, contract_class)) = btx.declared_contract_class() {
                            tracing::debug!("Declared class_hash={:#x}", class_hash.to_felt());
                            stats.declared_classes += 1;
                            execution_state.declared_classes.insert(class_hash, contract_class);
                        }
                    }
                    Err(err) => {
                        // These are the transactions that have errored but we can't revert them. It can be because of an internal server error, but
                        // errors during the execution of Declare and DeployAccount also appear here as they cannot be reverted.
                        // We reject them.
                        // Note that this is a big DoS vector.
                        tracing::error!(
                            "Rejected transaction {:#x} for unexpected error: {err:#}",
                            btx.tx_hash().to_felt()
                        );
                        stats.n_rejected += 1;
                    }
                }
            }
            l2_gas_consumed_block += stats.l2_gas_consumed;

            if self.replay_mode_enabled && !replay_executed_hashes.is_empty() {
                let block_n = execution_state.exec_ctx.block_number;
                if let Some(status) =
                    self.backend.replay_boundary_record_executed_hashes(block_n, &replay_executed_hashes)
                {
                    if let Some(mismatch) = status.mismatch {
                        tracing::warn!(
                            "replay_boundary_mismatch_after_execution block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} message={}",
                            block_n,
                            status.expected_tx_count,
                            status.executed_tx_count,
                            status.dispatched_tx_count,
                            status.reached_last_tx_hash,
                            mismatch
                        );
                    }
                }
            }

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

            let exec_result =
                super::BatchExecutionResult { executed_txs, blockifier_results, stats, emitted_at: StdInstant::now() };
            if has_txs_in_batch {
                tracing::debug!(
                    "executor_batch_execution_finished block_number={} txs_requested={} txs_executed={} txs_added_to_block={} txs_reverted={} txs_rejected={} batch_exec_duration_ms={} block_full={} first_tx_hash={} first_tx_type={}",
                    execution_state.exec_ctx.block_number,
                    exec_result.executed_txs.len(),
                    exec_result.stats.n_executed,
                    exec_result.stats.n_added_to_block,
                    exec_result.stats.n_reverted,
                    exec_result.stats.n_rejected,
                    exec_duration.as_secs_f64() * 1000.0,
                    block_full,
                    first_tx_hash,
                    first_tx_type
                );
            }
            if exec_result.stats.n_executed > 0
                && self.replies_sender.blocking_send(super::ExecutorMessage::BatchExecuted(exec_result)).is_err()
            {
                // Receiver closed
                break Ok(());
            }

            // End a block once we reached the block closing condition.
            // This transitions the state machine from ExecutorState::Executing to ExecutorState::NewBlock.

            let block_n = execution_state.exec_ctx.block_number;
            let now = Instant::now();
            let block_time_deadline_reached = now >= next_block_deadline;
            let replay_boundary_exists = self.replay_mode_enabled && self.replay_boundary_exists(block_n);
            let replay_boundary_met =
                if replay_boundary_exists { self.replay_boundary_is_met(block_n).unwrap_or(false) } else { false };

            let should_close = if replay_boundary_exists {
                force_close || replay_boundary_met
            } else {
                force_close || block_full || block_time_deadline_reached
            };

            if should_close {
                tracing::debug!(
                    "Ending block block_n={} (force_close={force_close}, block_full={block_full}, block_time_deadline_reached={block_time_deadline_reached}, replay_boundary_exists={replay_boundary_exists}, replay_boundary_met={replay_boundary_met})",
                    block_n,
                );
                let finalize_start = Instant::now();
                let block_exec_summary = execution_state.executor.finalize()?;
                let finalize_secs = finalize_start.elapsed().as_secs_f64();
                self.metrics.executor_finalize_duration.record(finalize_secs, &[]);
                self.metrics.executor_finalize_last.record(finalize_secs, &[]);
                tracing::debug!(
                    block_number = block_n,
                    finalize_ms = finalize_secs * 1000.0,
                    "executor_finalize_complete"
                );

                if self
                    .replies_sender
                    .blocking_send(super::ExecutorMessage::EndBlock(Box::new(block_exec_summary)))
                    .is_err()
                {
                    // Receiver closed
                    break Ok(());
                }
                tracing::debug!(
                    block_number = block_n,
                    force_close = force_close,
                    block_full = block_full,
                    block_time_deadline_reached = block_time_deadline_reached,
                    replay_boundary_exists = replay_boundary_exists,
                    replay_boundary_met = replay_boundary_met,
                    "executor_end_block_sent"
                );
                next_block_deadline = Instant::now() + block_time;
                let end_block_start = Instant::now();
                state = self.end_block(execution_state).context("Ending block")?;
                tracing::debug!(
                    end_block_ms = end_block_start.elapsed().as_secs_f64() * 1000.0,
                    "executor_end_block_state_transition_complete"
                );
                block_empty = true;
                force_close = false;
                if !replay_next_block_buffer.is_empty() {
                    to_exec.extend(mem::take(&mut replay_next_block_buffer));
                }
            }
        }
    }
}
