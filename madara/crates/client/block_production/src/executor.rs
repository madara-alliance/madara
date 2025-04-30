use crate::util::{create_execution_context, BatchToExecute, BlockExecutionContext, ExecutionStats};
use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::{TransactionExecutor, TransactionExecutorResult},
    execution::contract_class::ContractClass,
    state::{
        cached_state::{StateMaps, StorageEntry},
        state_api::State,
    },
    transaction::objects::TransactionExecutionInfo,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_exec::{execution::TxInfo, LayeredStateAdaptor, MadaraBackendExecutionExt};
use mc_mempool::L1DataProvider;
use mp_convert::{Felt, ToFelt};
use starknet_api::{
    core::{ClassHash, ContractAddress, PatriciaKey},
    state::StorageKey,
};
use std::{any::Any, collections::HashMap, mem, panic::AssertUnwindSafe, sync::Arc};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

#[derive(Debug)]
pub(crate) enum ExecutorMessage {
    StartNewBlock {
        /// Used to add the block_n-10 block hash table entry to the state diff.
        initial_state_diffs_storage: HashMap<StorageEntry, Felt>,
        /// The proto-header. It's exactly like PendingHeader, but it does not have the parent_block_hash field because it's not known yet.
        exec_ctx: BlockExecutionContext,
    },
    BatchExecuted(BatchExecutionResult),
    EndBlock,
}

#[derive(Default, Debug)]
pub(crate) struct BatchExecutionResult {
    pub executed_txs: BatchToExecute,
    pub blockifier_results: Vec<TransactionExecutorResult<TransactionExecutionOutput>>,
    pub stats: ExecutionStats,
}

/// Receiver for the stop condition of the executor thread.
pub(crate) struct StopErrorReceiver(oneshot::Receiver<Result<anyhow::Result<()>, Box<dyn Any + Send + 'static>>>);
impl StopErrorReceiver {
    pub async fn recv(&mut self) -> anyhow::Result<()> {
        match (&mut self.0).await {
            Ok(Ok(res)) => res,
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => Ok(()), // channel closed
        }
    }
}

/// Handle to used to talk with the executor thread.
pub struct ExecutorHandle {
    /// Input transactions need to be sent to this sender channel.
    /// Closing this channel will tell the executor thread to stop.
    pub send_batch: Option<mpsc::Sender<BatchToExecute>>,
    /// Receive the resulting Result of the thread.
    pub stop: StopErrorReceiver,
    /// Channel with the replies from the executor thread.
    pub replies: mpsc::Receiver<ExecutorMessage>,
}

// TODO(blockifier_update): When updating blockifier, remove the following lines! (TransactionExecutionOutput and blockifier_v0_8_0_fix_results).
//
// The reason is, in the new blockifier update, TransactionExecutor::execute_txs does not return `Vec<TransactionExecutorResult<TransactionExecutionInfo>>`
// anymore. Instead, it returns `Vec<TransactionExecutorResult<TransactionExecutionOutput>>`, where they define `TransactionExecutionOutput` as the type alias
// `type TransactionExecutionOutput = (TransactionExecutionInfo, StateMaps)`. This new StateMaps field is is the state diff produced for the corresponding TransactionExecutionInfo.
// This version of the crate was made to use this new `TransactionExecutionOutput`; but since we decided to split the blockifier update and the block production refactor into separate
// PRs, we need this glue for the time being.
//
// This is completely temporary.
type TransactionExecutionOutput = (TransactionExecutionInfo, StateMaps);
fn blockifier_v0_8_0_fix_results(
    state: &mut ExecutorStateExecuting,
    results: Vec<TransactionExecutorResult<TransactionExecutionInfo>>,
) -> anyhow::Result<Vec<TransactionExecutorResult<TransactionExecutionOutput>>> {
    // HACK: We can't get the state diff for individual transactions, nor can we get the state diff for the current batch.
    // We need to cheat by getting the state diff for the whole block, and we put that state diff as the state diff of the first transaction of the batch.
    // This works because the upstream task will be merging the same state diff multiple times, which always results in the same state diff.
    let state = state.executor.block_state.as_mut().expect("Block state taken");

    let mut added = false;
    // It is not an error if added remains false, it just means there was no tx added to the block in this batch.
    // No need to output state diff in that case.
    results
        .into_iter()
        .map(|r| match r {
            TransactionExecutorResult::Ok(res) => anyhow::Ok(TransactionExecutorResult::Ok((
                res,
                if !added {
                    added = true;
                    state.to_state_diff().context("Converting to state diff")?
                } else {
                    StateMaps::default()
                },
            ))),
            TransactionExecutorResult::Err(r) => anyhow::Ok(TransactionExecutorResult::Err(r)),
        })
        .collect()
}

struct ExecutorStateExecuting {
    exec_ctx: BlockExecutionContext,
    /// Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    /// bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    /// we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<LayeredStateAdaptor>,
    declared_classes: HashMap<ClassHash, ContractClass>,
}

struct ExecutorStateNewBlock {
    /// Keep the cached adaptor around to keep the cache around.
    state_adaptor: LayeredStateAdaptor,
}

/// Note: The reason this exists is because we want to create the new block execution context (meaning, the block header) as late as possible, as to have
/// the best gas prices. This is especially important when the no_empty_block configuration is enabled, as otherwise we would end up:
/// - Creating a new execution context, using the current gas prices.
/// - Waiting for a transaction to arrive.... potentially for a very, very long time..
/// - Transaction arrives, we execute it and close the block, as the block_time is reached.
///
/// At that point, the gas prices would be all wrong! In order to support no_empty_block correctly, we have to delay execution context creation
/// until the first transaction has arrived.
enum ExecutorState {
    /// A block has been started.
    Executing(ExecutorStateExecuting),
    /// Intermediate state, we do not have initialized the execution yet.
    NewBlock(ExecutorStateNewBlock),
}

/// Executor runs on a separate thread, as to avoid having tx popping, block closing etc. take precious time away that could
/// be spent executing the next tick instead.
/// This thread becomes the blockifier executor scheduler thread (via TransactionExecutor), which will internally spawn worker threads.
pub(crate) struct Executor {
    backend: Arc<MadaraBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,

    incoming_batches: mpsc::Receiver<BatchToExecute>,
    replies_sender: mpsc::Sender<ExecutorMessage>,

    /// See `take_tx_batch`. When the mempool is empty, we will not be getting transactions.
    /// We still potentially want to emit empty blocks based on the block_time deadline.
    wait_rt: tokio::runtime::Runtime,
}

impl Executor {
    /// Create the executor thread and returns a handle to it.
    pub(crate) fn start(
        backend: Arc<MadaraBackend>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> anyhow::Result<ExecutorHandle> {
        // buffer is 1.
        let (sender, recv) = mpsc::channel(1);
        let (replies_sender, replies_recv) = mpsc::channel(100);
        let (stop_sender, stop_recv) = oneshot::channel();

        let executor = Self {
            incoming_batches: recv,
            replies_sender,
            backend,
            l1_data_provider,
            wait_rt: tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .context("Building tokio runtime")?,
        };
        std::thread::Builder::new()
            .name("executor".into())
            .spawn(move || stop_sender.send(std::panic::catch_unwind(AssertUnwindSafe(move || executor.run()))))
            .context("Error when spawning thread")?;

        Ok(ExecutorHandle { send_batch: Some(sender), replies: replies_recv, stop: StopErrorReceiver(stop_recv) })
    }

    /// Returns None when the channel is closed.
    /// We want to close down the thread in that case.
    fn wait_take_tx_batch(&mut self, deadline: Option<Instant>, should_wait: bool) -> Option<BatchToExecute> {
        if let Ok(batch) = self.incoming_batches.try_recv() {
            return Some(batch);
        }

        if !should_wait {
            return Some(Default::default());
        }

        tracing::debug!("Waiting for batch. block_time_deadline={}", deadline.is_some());

        let res = if let Some(deadline) = deadline {
            // nb: tokio has blocking_recv, but no blocking_recv_timeout? this kinda sucks :(
            // especially because they do have it implemented in send_timeout and internally, they just have not exposed the
            // function.
            // Should be fine, as we optimistically try_recv above and we should only hit this when we actually have to wait.
            // nb.2: use an async block here, as timeout_at needs a runtime to be available on creation.
            self.wait_rt.block_on(async { tokio::time::timeout_at(deadline, self.incoming_batches.recv()).await })
        } else {
            Ok(self.incoming_batches.blocking_recv())
        };

        match res {
            Ok(Some(el)) => {
                tracing::debug!("Got new batch with {} transactions.", el.len());
                Some(el)
            }
            Ok(None) => {
                tracing::debug!("Batch channel closed.");
                None
            }
            Err(_timed_out) => {
                tracing::debug!("Waiting for batch timed out.");
                Some(Default::default())
            }
        }
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
    fn end_block(&mut self, state: &mut ExecutorStateExecuting) -> anyhow::Result<ExecutorState> {
        let mut cached_state = state.executor.block_state.take().expect("Executor block state already taken");

        let state_diff = cached_state.to_state_diff().context("Cannot make state diff")?;
        let mut cached_adaptor = cached_state.state;
        cached_adaptor.finish_block(state_diff, mem::take(&mut state.declared_classes))?;

        Ok(ExecutorState::NewBlock(ExecutorStateNewBlock { state_adaptor: cached_adaptor }))
    }

    /// Returns the initial state diff storage too. It is used to create the StartNewBlock message and transition to ExecutorState::Executing.
    fn create_execution_state(
        &mut self,
        state: ExecutorStateNewBlock,
    ) -> anyhow::Result<(ExecutorStateExecuting, HashMap<StorageEntry, Felt>)> {
        let exec_ctx = create_execution_context(&self.l1_data_provider, &self.backend, state.state_adaptor.block_n());

        // Create the TransactionExecution, but reuse the layered_state_adaptor.
        let mut executor =
            self.backend.new_executor_for_block_production(state.state_adaptor, exec_ctx.to_blockifier()?)?;

        // Prepare the block_n_min_10 state diff entry.
        let mut state_maps_storages = HashMap::default();

        if let Some((block_n_min_10, block_hash_n_min_10)) = self.wait_for_hash_of_block_min_10(exec_ctx.block_n)? {
            let contract_address = ContractAddress(PatriciaKey::try_from(Felt::ONE).expect("Const conversion"));
            let key = StorageKey::try_from(Felt::from(block_n_min_10)).context("Block_n overflows storage key")?;
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
        Ok((ExecutorStateExecuting { exec_ctx, executor, declared_classes: HashMap::new() }, state_maps_storages))
    }

    fn initial_state(&self) -> anyhow::Result<ExecutorState> {
        Ok(ExecutorState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: LayeredStateAdaptor::new(Arc::clone(&self.backend))?,
        }))
    }

    fn run(mut self) -> anyhow::Result<()> {
        let batch_size = self.backend.chain_config().block_production_concurrency.batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        // Initial state is ExecutorState::NewBlock, we don't yet have an execution state.
        let mut state = self.initial_state().context("Creating executor initial state")?;

        // The batch of transactions to execute.
        let mut to_exec = BatchToExecute::with_capacity(batch_size);

        let mut next_block_deadline = Instant::now() + block_time;
        let mut block_empty = true;

        // The goal here is to do the least possible between batches, as to maximize CPU usage. Any millisecond spent
        //  outside of `TransactionExecutor::execute_txs` is a millisecond where we could have used every CPU cores, but are using only one.
        // `blockifier` isn't really well optimized in this regard, but since we can't easily change its code (maybe we should?) we're
        //  still optimizing everything we have a hand on here in madara.
        loop {
            // Take transactions to execute.
            if to_exec.len() < batch_size {
                let wait_deadline = if block_empty && no_empty_blocks { None } else { Some(next_block_deadline) };
                // should_wait: We don't want to wait if we already have transactions to process - but we would still like to fill up our batch if possible.
                let Some(taken) = self.wait_take_tx_batch(wait_deadline, /* should_wait */ to_exec.is_empty()) else {
                    return Ok(()); // Channel closed. Exit gracefully.
                };

                to_exec.extend(taken);
            }

            // Create a new execution state (new block) if it does not already exist.
            // This transitions the state machine from ExecutorState::NewBlock to ExecutorState::Executing, and
            // creates the blockifier TransactionExecutor.
            let execution_state = match state {
                ExecutorState::Executing(ref mut executor_state_executing) => executor_state_executing,
                ExecutorState::NewBlock(state_new_block) => {
                    // Create new execution state.
                    let (execution_state, initial_state_diffs_storage) =
                        self.create_execution_state(state_new_block).context("Creating execution state")?;

                    tracing::debug!("Starting new block, block_n={}", execution_state.exec_ctx.block_n);
                    if self
                        .replies_sender
                        .blocking_send(ExecutorMessage::StartNewBlock {
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
                    state = ExecutorState::Executing(execution_state);
                    let ExecutorState::Executing(execution_state) = &mut state else { unreachable!() };
                    execution_state
                }
            };

            let exec_start_time = Instant::now();

            // Execute the transactions.
            let blockifier_results = execution_state.executor.execute_txs(&to_exec.txs);

            // TODO(blockifier_update): Remove the following line when updating blockifier.
            let blockifier_results = blockifier_v0_8_0_fix_results(execution_state, blockifier_results)?;

            let exec_duration = exec_start_time.elapsed();

            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            let block_full = blockifier_results.len() < to_exec.len();

            let executed_txs = to_exec.remove_n_front(blockifier_results.len()); // Remove the used txs.

            let mut stats = ExecutionStats::default();
            stats.n_batches += 1;
            stats.n_executed += executed_txs.len();
            stats.exec_duration += exec_duration;

            // We does not process the results, it just inspect them for logging stats, and figure out which classes were declared.
            // Results are processed async, outside of the executor.
            for (btx, res) in executed_txs.txs.iter().zip(blockifier_results.iter()) {
                match res {
                    Ok((execution_info, _state_diff)) => {
                        tracing::trace!("Successful execution of transaction {:#x}", btx.tx_hash().to_felt());

                        stats.n_added_to_block += 1;
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

            tracing::debug!("Finished batch execution.");
            tracing::debug!("Stats: {:?}", stats);
            tracing::debug!("Weights: {:?}", execution_state.executor.bouncer.get_accumulated_weights());
            tracing::debug!("Block now full: {:?}", block_full);

            let exec_result = BatchExecutionResult { executed_txs, blockifier_results, stats };
            if self.replies_sender.blocking_send(ExecutorMessage::BatchExecuted(exec_result)).is_err() {
                // Receiver closed
                break Ok(());
            }

            // End a block once we reached the block closing condition.
            // This transitions the state machine from ExecutorState::Executing to ExecutorState::NewBlock.

            let now = Instant::now();
            let block_time_deadline_reached = now >= next_block_deadline;
            if block_full || block_time_deadline_reached {
                tracing::debug!(
                    "Ending block block_n={} (block_full={block_full}, block_time_deadline_reached={block_time_deadline_reached})",
                    execution_state.exec_ctx.block_n,
                );

                if self.replies_sender.blocking_send(ExecutorMessage::EndBlock).is_err() {
                    // Receiver closed
                    break Ok(());
                }
                next_block_deadline = Instant::now() + block_time;
                state = self.end_block(execution_state).context("Ending block")?;
                block_empty = true;
            }
        }
    }
}
