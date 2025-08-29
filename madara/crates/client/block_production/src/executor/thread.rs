//! Executor thread internal logic.

use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::TransactionExecutor,
    state::{cached_state::StorageEntry, state_api::State},
};
use futures::future::OptionFuture;
use starknet_api::contract_class::ContractClass;
use starknet_api::core::ClassHash;
use std::{
    collections::{HashMap, HashSet},
    mem,
    sync::Arc,
};
use tokio::{
    sync::{broadcast, mpsc},
    time::Instant,
};

use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_exec::{execution::TxInfo, LayeredStateAdapter, MadaraBackendExecutionExt};
use mp_convert::{Felt, ToFelt};

use crate::util::{create_execution_context, BatchToExecute, BlockExecutionContext, ExecutionStats};

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
    ) -> anyhow::Result<Self> {
        Ok(Self {
            backend,
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
    fn wait_take_tx_batch(&mut self, deadline: Option<Instant>, should_wait: bool) -> WaitTxBatchOutcome {
        if let Ok(batch) = self.incoming_batches.try_recv() {
            return WaitTxBatchOutcome::Batch(batch);
        }

        if let Ok(cmd) = self.commands.try_recv() {
            return WaitTxBatchOutcome::Command(cmd);
        }

        if !should_wait {
            return WaitTxBatchOutcome::Batch(Default::default());
        }

        tracing::debug!("Waiting for batch. until_block_time_deadline={}", deadline.is_some());

        // nb: tokio has blocking_recv, but no blocking_recv_timeout? this kinda sucks :(
        // especially because they do have it implemented in send_timeout and internally, they just have not exposed the
        // function.
        // Should be fine, as we optimistically try_recv above and we should only hit this when we actually have to wait.
        // nb.2: use an async block here, as timeout_at needs a runtime to be available on creation.
        self.wait_rt.block_on(async {
            tokio::select! {
                Some(cmd) = self.commands.recv() => {
                    tracing::debug!("Got cmd {cmd:?}.");
                    WaitTxBatchOutcome::Command(cmd)
                }
                _ = OptionFuture::from(deadline.map(tokio::time::sleep_until)) => {
                    tracing::debug!("Waiting for batch timed out.");
                    WaitTxBatchOutcome::Batch(Default::default())
                }
                el = self.incoming_batches.recv() => match el {
                    Some(el) => {
                        tracing::debug!("Got new batch with {} transactions.", el.len());
                        WaitTxBatchOutcome::Batch(el)
                    }
                    None => {
                        tracing::debug!("Batch channel closed.");
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
            self.backend
                .get_block_hash(&DbBlockId::Number(block_n_min_10))
                .context("Getting block hash of block_n - 10")
        };

        // Optimistically get the hash from database without subscribing to the closed_blocks channel.
        if let Some(block_hash) = get_hash_from_db()? {
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            tracing::debug!("Waiting on block_n={} to get closed. (current={})", block_n_min_10, block_n);
            loop {
                let mut receiver = self.backend.subscribe_closed_blocks();
                // We need to re-query the DB here since the it is possible for the block hash to have arrived just in between.
                if let Some(block_hash) = get_hash_from_db()? {
                    break Ok(Some((block_n_min_10, block_hash)));
                }
                tracing::debug!("Waiting for hash of block_n-10.");
                match receiver.blocking_recv() {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        anyhow::bail!("Backend latest block channel closed")
                    }
                }
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
        previous_l2_gas_used: u64,
    ) -> anyhow::Result<(ExecutorStateExecuting, HashMap<StorageEntry, Felt>)> {
        let previous_l2_gas_price = state.state_adaptor.latest_gas_prices().strk_l2_gas_price;
        let exec_ctx = create_execution_context(
            &self.backend,
            state.state_adaptor.block_n(),
            previous_l2_gas_price,
            previous_l2_gas_used,
        )?;

        println!("Execution context created : {:?}", exec_ctx);

        // Create the TransactionExecution, but reuse the layered_state_adapter.
        let mut executor =
            self.backend.new_executor_for_block_production(state.state_adaptor, exec_ctx.to_blockifier()?)?;

        // Prepare the block_n_min_10 state diff entry.
        let mut state_maps_storages = HashMap::default();

        if let Some((block_n_min_10, block_hash_n_min_10)) = self.wait_for_hash_of_block_min_10(exec_ctx.block_n)? {
            let contract_address = 1u64.into();
            let key = block_n_min_10.into();
            executor
                .block_state
                .as_mut()
                .expect("Blockifier block context has been taken")
                .set_storage_at(contract_address, key, block_hash_n_min_10)
                .context("Cannot set storage value in cache")?;
            state_maps_storages.insert((contract_address, key), block_hash_n_min_10);

            tracing::debug!(
                "State diff inserted {:#x} {:#x} => {block_hash_n_min_10:#x}",
                contract_address.to_felt(),
                key.to_felt()
            );
        }
        Ok((
            ExecutorStateExecuting {
                exec_ctx,
                executor,
                consumed_l1_to_l2_nonces: state.consumed_l1_to_l2_nonces,
                declared_classes: HashMap::new(),
            },
            state_maps_storages,
        ))
    }

    fn initial_state(&self) -> anyhow::Result<ExecutorThreadState> {
        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: LayeredStateAdapter::new(Arc::clone(&self.backend))?,
            consumed_l1_to_l2_nonces: HashSet::new(),
        }))
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        let batch_size = self.backend.chain_config().block_production_concurrency.batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        // Initial state is ExecutorState::NewBlock, we don't yet have an execution state.
        let mut state = self.initial_state().context("Creating executor initial state")?;

        // The batch of transactions to execute.
        let mut to_exec = BatchToExecute::with_capacity(batch_size);

        let mut next_block_deadline = Instant::now() + block_time;
        let mut force_close = false;
        let mut block_empty = true;
        let mut l2_gas_consumed_block = 0;

        tracing::debug!("Starting executor thread.");

        // The goal here is to do the least possible between batches, as to maximize CPU usage. Any millisecond spent
        //  outside of `TransactionExecutor::execute_txs` is a millisecond where we could have used every CPU cores, but are using only one.
        // `blockifier` isn't really well optimized in this regard, but since we can't easily change its code (maybe we should?) we're
        //  still optimizing everything we have a hand on here in madara.
        loop {
            // Take transactions to execute.
            if to_exec.len() < batch_size {
                let wait_deadline = if block_empty && no_empty_blocks { None } else { Some(next_block_deadline) };
                // should_wait: We don't want to wait if we already have transactions to process - but we would still like to fill up our batch if possible.

                let taken = match self.wait_take_tx_batch(wait_deadline, /* should_wait */ to_exec.is_empty()) {
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
                    WaitTxBatchOutcome::Exit => return Ok(()),
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
                    let (execution_state, initial_state_diffs_storage) = self
                        .create_execution_state(state_new_block, l2_gas_consumed_block)
                        .context("Creating execution state")?;
                    l2_gas_consumed_block = 0;

                    tracing::debug!("Starting new block, block_n={}", execution_state.exec_ctx.block_n);
                    if self
                        .replies_sender
                        .blocking_send(super::ExecutorMessage::StartNewBlock {
                            initial_state_diffs_storage,
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

            let exec_start_time = Instant::now();

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
            // Results are processed async, outside of the executor.
            for (btx, res) in executed_txs.txs.iter().zip(blockifier_results.iter()) {
                match res {
                    Ok((execution_info, _state_diff)) => {
                        tracing::trace!("Successful execution of transaction {:#x}", btx.tx_hash().to_felt());

                        stats.n_added_to_block += 1;
                        stats.l2_gas_consumed += execution_info.receipt.gas.l2_gas.0;
                        block_empty = false;
                        if execution_info.is_reverted() {
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

            tracing::debug!("Finished batch execution.");
            tracing::debug!("Stats: {:?}", stats);
            tracing::debug!(
                "Weights: {:?}",
                execution_state.executor.bouncer.lock().expect("Bouncer lock poisoned").get_accumulated_weights()
            );
            tracing::debug!("Block now full: {:?}", block_full);

            let exec_result = super::BatchExecutionResult { executed_txs, blockifier_results, stats };
            if self.replies_sender.blocking_send(super::ExecutorMessage::BatchExecuted(exec_result)).is_err() {
                // Receiver closed
                break Ok(());
            }

            // End a block once we reached the block closing condition.
            // This transitions the state machine from ExecutorState::Executing to ExecutorState::NewBlock.

            let now = Instant::now();
            let block_time_deadline_reached = now >= next_block_deadline;
            if force_close || block_full || block_time_deadline_reached {
                tracing::debug!(
                    "Ending block block_n={} (force_close={force_close}, block_full={block_full}, block_time_deadline_reached={block_time_deadline_reached})",
                    execution_state.exec_ctx.block_n,
                );

                if self.replies_sender.blocking_send(super::ExecutorMessage::EndBlock).is_err() {
                    // Receiver closed
                    break Ok(());
                }
                next_block_deadline = Instant::now() + block_time;
                state = self.end_block(execution_state).context("Ending block")?;
                block_empty = true;
                force_close = false;
            }
        }
    }
}
