use crate::util::{
    create_blockifier_executor, create_blockifier_state_adaptor, create_execution_context, BlockExecutionContext,
};
use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::{TransactionExecutionOutput, TransactionExecutor, TransactionExecutorResult},
    execution::contract_class::RunnableCompiledClass,
    state::{cached_state::StorageEntry, state_api::State},
    transaction::transaction_execution::Transaction,
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_exec::CachedStateAdaptor;
use mc_mempool::L1DataProvider;
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use starknet_api::{
    core::{ClassHash, ContractAddress, PatriciaKey},
    state::StorageKey,
};
use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    mem,
    panic::AssertUnwindSafe,
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

#[derive(Default, Debug)]
pub(crate) struct BatchToExecute {
    pub txs: Vec<Transaction>,
    pub additional_info: VecDeque<AdditionalTxInfo>,
}

impl BatchToExecute {
    pub fn with_capacity(cap: usize) -> Self {
        Self { txs: Vec::with_capacity(cap), additional_info: VecDeque::with_capacity(cap) }
    }

    pub fn extend(&mut self, other: Self) {
        self.txs.extend(other.txs);
        self.additional_info.extend(other.additional_info);
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn push(&mut self, tx: Transaction, additional_info: AdditionalTxInfo) {
        self.txs.push(tx);
        self.additional_info.push_back(additional_info);
    }

    pub fn remove_n_front(&mut self, n_to_remove: usize) -> BatchToExecute {
        let txs = self.txs.drain(..n_to_remove).collect();
        let additional_info = self.additional_info.drain(..n_to_remove).collect();
        BatchToExecute { txs, additional_info }
    }
}

#[derive(Debug)]
pub(crate) struct AdditionalTxInfo {
    pub tx_hash: Felt,
    pub declared_class: Option<Box<ConvertedClass>>,
}

#[derive(Debug)]
pub(crate) enum ExecutorMessage {
    StartNewBlock {
        /// Used to add the block_n-10 entry to the state diff.
        initial_state_diffs_storage: HashMap<StorageEntry, Felt>,
        /// The proto-header. It's exactly like PendingHeader, but it does not have the parent_block_hash field because it's not known yet.
        exec_ctx: BlockExecutionContext,
    },
    BatchExecuted(BatchExecutionResult),
    EndBlock,
}

#[derive(Default, Debug)]
pub(crate) struct BatchExecutionResult {
    // // All executed transactions, including the rejected ones.
    // pub executed: Vec<Felt>,

    // pub new_events: Vec<EventWithTransactionHash>,
    // pub new_transactions: Vec<TransactionWithReceipt>,
    // pub new_state_diffs: StateMaps,
    // pub new_declared_classes: Vec<ConvertedClass>,

    // pub stats: ContinueBlockStats,
    pub exec_duration: Duration,
    pub executed_txs: BatchToExecute,
    pub blockifier_results: Vec<TransactionExecutorResult<TransactionExecutionOutput>>,
}

pub struct StopErrorReceiver(oneshot::Receiver<Result<anyhow::Result<()>, Box<dyn Any + Send + 'static>>>);
impl StopErrorReceiver {
    pub async fn recv(&mut self) -> anyhow::Result<()> {
        match (&mut self.0).await {
            Ok(Ok(res)) => res,
            Ok(Err(panic)) => std::panic::resume_unwind(panic),
            Err(_) => Ok(()), // channel closed
        }
    }
}

pub struct ExecutorHandle {
    pub send_batch: Option<mpsc::Sender<BatchToExecute>>,
    pub stop: StopErrorReceiver,
    pub replies: mpsc::Receiver<ExecutorMessage>,
}

struct ExecutorStateExecuting {
    exec_ctx: BlockExecutionContext,
    /// Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    /// bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    /// we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<CachedStateAdaptor>,
    declared_classes: Vec<ConvertedClass>,
}

struct ExecutorStateNewBlock {
    /// Keep the cached adaptor around to keep the cache around.
    cached_adaptor: CachedStateAdaptor,
    /// Used to add the block_n-10 entry to the state diff.
    /// The reason we're getting it during the NewBlock state and not at the beginning of the Executing state, is because
    /// if block closing is late by a whole 10 blocks, we'll end up waiting - and we want to wait during the NewBlock state,
    /// as to delay the gas price fetching accordingly.
    block_n_min_10_entry: Option<(u64, Felt)>,
    latest_block_n: Option<u64>,
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
pub struct Executor {
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
    pub fn create(
        backend: Arc<MadaraBackend>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> anyhow::Result<ExecutorHandle> {
        // buffer is 1.
        let (sender, recv) = mpsc::channel(1);
        let (replies_sender, replies_recv) = mpsc::channel(10);
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

        tracing::debug!("Waiting for batch. Deadline={:?}", deadline);

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

        if let Some(block_hash) = get_hash_from_db()? {
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            // only subscribe when block is not found
            tracing::debug!("Waiting on block_n={} to get closed. (current={})", block_n_min_10, block_n);
            loop {
                let mut receiver = self.backend.subscribe_closed_blocks();
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

        let state_maps = cached_state.to_state_diff().context("Cannot make state diff")?.state_maps;
        let declared_classes: HashMap<_, _> = mem::take(&mut state.declared_classes)
            .into_iter()
            .map(|class| {
                let class_hash = class.class_hash();
                (&class)
                    .try_into()
                    .map_err(|e| anyhow::anyhow!("Cannot convert to blockifier runnable class: {e}"))
                    .map(|c: RunnableCompiledClass| (ClassHash(class_hash), c))
            })
            .collect::<Result<_, _>>()?;

        let block_n = state.exec_ctx.block_n;

        let block_n_min_10_entry = self.wait_for_hash_of_block_min_10(block_n + 1)?;

        // todo: we should provide a better api here :/
        let mut cached_adaptor = cached_state.state;
        cached_adaptor.inner.block_number = block_n + 1;
        cached_adaptor.push_to_cache(block_n, state_maps, declared_classes);
        if let Some(block_n) = self.backend.get_latest_block_n().context("Getting latest block_n")? {
            cached_adaptor.remove_cache_older_than(block_n);
            cached_adaptor.inner.on_top_of_block_id = Some(DbBlockId::Number(block_n));
        }

        Ok(ExecutorState::NewBlock(ExecutorStateNewBlock {
            cached_adaptor,
            block_n_min_10_entry,
            latest_block_n: Some(block_n),
        }))
    }

    /// Returns the initial state diff storage too. It is used to create the new block message.
    fn create_execution_state(
        &mut self,
        state: ExecutorStateNewBlock,
    ) -> anyhow::Result<(ExecutorStateExecuting, HashMap<StorageEntry, Felt>)> {
        let exec_ctx = create_execution_context(&self.l1_data_provider, &self.backend, state.latest_block_n);
        let mut executor = create_blockifier_executor(state.cached_adaptor, &self.backend, &exec_ctx)?;

        let mut state_maps_storages = HashMap::default();

        if let Some((block_n_min_10, block_hash_n_min_10)) = state.block_n_min_10_entry {
            let contract_address = ContractAddress(PatriciaKey::ONE);
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
        Ok((ExecutorStateExecuting { exec_ctx, executor, declared_classes: vec![] }, state_maps_storages))
    }

    fn initial_state(&self) -> anyhow::Result<ExecutorState> {
        let on_block_n = self.backend.get_latest_block_n().context("Getting latest block in database")?;
        let block_n = on_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);

        let cached_adaptor = create_blockifier_state_adaptor(&self.backend, on_block_n);

        Ok(ExecutorState::NewBlock(ExecutorStateNewBlock {
            cached_adaptor,
            block_n_min_10_entry: self.wait_for_hash_of_block_min_10(block_n)?,
            latest_block_n: on_block_n,
        }))
    }

    fn run(mut self) -> anyhow::Result<()> {
        let mut state = self.initial_state().context("Creating executor initial state")?;

        let mut block_empty = true;

        let batch_size = self.backend.chain_config().execution_batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        let mut to_exec = BatchToExecute::with_capacity(batch_size);

        let mut next_block_deadline = Instant::now() + block_time;

        loop {
            // Take transactions to execute.
            if to_exec.len() < batch_size {
                // We don't want to wait if we still have transactions to process - but we would still like to fill up out batch if possible.

                let wait_deadline = if block_empty && no_empty_blocks { None } else { Some(next_block_deadline) };
                let Some(taken) = self.wait_take_tx_batch(wait_deadline, /* should_wait */ to_exec.is_empty()) else {
                    return Ok(()); // Channel closed. Exit gracefully.
                };

                to_exec.extend(taken);
            }

            // Create execution state if it does not already exist.
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

                    // I wish rust had a better way to do that :/
                    state = ExecutorState::Executing(execution_state);
                    let ExecutorState::Executing(execution_state) = &mut state else { unreachable!() };
                    execution_state
                }
            };

            let exec_start_time = Instant::now();

            // Execute the transactions.
            let blockifier_results = execution_state.executor.execute_txs(&to_exec.txs);
            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            let block_full = blockifier_results.len() < to_exec.len();

            if blockifier_results.iter().any(|r| r.is_ok()) {
                block_empty = false;
            }

            let executed_txs = to_exec.remove_n_front(blockifier_results.len()); // remove the used txs

            let exec_duration = exec_start_time.elapsed();

            tracing::debug!("Weights: {:?}", execution_state.executor.bouncer.get_accumulated_weights());
            tracing::debug!("Block now full: {:?}", block_full);

            let exec_result = BatchExecutionResult { exec_duration, executed_txs, blockifier_results };
            if self.replies_sender.blocking_send(ExecutorMessage::BatchExecuted(exec_result)).is_err() {
                // Receiver closed
                break Ok(());
            }

            let now = Instant::now();
            let block_time_deadline_reached = now >= next_block_deadline;
            if block_full || block_time_deadline_reached {
                tracing::debug!("Ending block block_n={}", execution_state.exec_ctx.block_n);

                if self.replies_sender.blocking_send(ExecutorMessage::EndBlock).is_err() {
                    // Receiver closed
                    break Ok(());
                }
                next_block_deadline = Instant::now() + block_time;
                block_empty = true;
                state = self.end_block(execution_state).context("Ending block")?;
            }
        }
    }
}
