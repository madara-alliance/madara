use crate::{
    util::{create_blockifier_executor, create_blockifier_state_adaptor, BlockExecutionContext},
    ContinueBlockStats,
};
use anyhow::Context;
use blockifier::{
    blockifier::transaction_executor::TransactionExecutor,
    execution::contract_class::RunnableCompiledClass,
    state::{cached_state::StateMaps, state_api::State},
};
use mc_db::{db_block_id::DbBlockId, MadaraBackend};
use mc_exec::{BlockifierStateAdapter, CachedStateAdaptor};
use mc_mempool::{L1DataProvider, MempoolTransaction};
use mp_block::TransactionWithReceipt;
use mp_class::ConvertedClass;
use mp_convert::{Felt, ToFelt};
use mp_receipt::{from_blockifier_execution_info, EventWithTransactionHash};
use mp_transactions::TransactionWithHash;
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
};
use tokio::{
    sync::{broadcast, mpsc, oneshot},
    time::Instant,
};

#[derive(Default, Debug)]
pub(crate) struct BatchReply {
    /// When set, the current block needs to be closed and this value is the new pending block header.
    /// All of the fields in this reply will refer to the new pending block.
    pub new_execution_context: Option<BlockExecutionContext>,

    // List of executed transactions, including the rejected ones.
    pub executed: Vec<Felt>,

    pub new_events: Vec<EventWithTransactionHash>,
    pub new_transactions: Vec<TransactionWithReceipt>,
    pub new_state_diffs: StateMaps,
    pub new_declared_classes: Vec<ConvertedClass>,

    pub stats: ContinueBlockStats,
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
    pub send_batch: Option<mpsc::Sender<Vec<MempoolTransaction>>>,
    pub stop: StopErrorReceiver,
    pub replies: mpsc::Receiver<BatchReply>,
}

/// Executor runs on a separate thread, as to avoid having tx popping, block closing etc. take precious time away that could
/// be spent executing the next tick instead.
/// This thread becomes the blockifier executor scheduler thread (via TransactionExecutor), which will internally spawn worker threads.
pub struct Executor {
    backend: Arc<MadaraBackend>,
    l1_data_provider: Arc<dyn L1DataProvider>,

    incoming_batches: mpsc::Receiver<Vec<MempoolTransaction>>,
    replies_sender: mpsc::Sender<BatchReply>,

    // Note: We have a special StateAdaptor here. This is because saving the block to the database can actually lag a
    // bit behind our execution. As such, any change that we make will need to be cached in our state adaptor so that
    // we can be sure the state of the last block is always visible to the new one.
    executor: TransactionExecutor<CachedStateAdaptor>,
    on_block_n: Option<u64>,
    declared_classes: Vec<ConvertedClass>,
    new_exec_ctx: Option<BlockExecutionContext>,

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

        let on_block_n = backend.get_latest_block_n().context("Getting latest block_n")?;

        let (blockifier_executor, new_exec_ctx) = create_blockifier_executor(
            create_blockifier_state_adaptor(&backend, on_block_n),
            &backend,
            &l1_data_provider,
            on_block_n,
        )?;

        let executor = Self {
            incoming_batches: recv,
            replies_sender,
            executor: blockifier_executor,
            new_exec_ctx: Some(new_exec_ctx),
            backend,
            l1_data_provider,
            on_block_n,
            declared_classes: vec![],
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
    fn wait_take_tx_batch(&mut self, deadline: Instant) -> Option<Vec<MempoolTransaction>> {
        if let Ok(batch) = self.incoming_batches.try_recv() {
            return Some(batch);
        }

        // nb: tokio has blocking_recv, but no blocking_recv_timeout? this kinda sucks :(
        // especially because they do have it implemented in send_timeout and internally, they just have not exposed the
        // function.
        // Should be fine, as we optimistically try_recv above and we should only hit this when we actually have to wait.
        let Some(batch) =
            self.wait_rt.block_on(tokio::time::timeout_at(deadline, self.incoming_batches.recv())).unwrap_or_default()
        else {
            // Channel closed.
            return None;
        };
        Some(batch)
    }

    // Returns None when executing the first 10 blocks, or current_block_n-10 and its block hash otherwise.
    // Will wait if the block is not yet in DB.
    fn wait_for_hash_of_block_min_10(&self) -> anyhow::Result<Option<(u64, Felt)>> {
        let block_n = self.on_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
        let Some(block_n_min_10) = block_n.checked_sub(10) else { return Ok(None) };

        let get_hash_from_db = || {
            self.backend
                .get_block_hash(&DbBlockId::Number(block_n_min_10))
                .context("Getting block hash of block_n - 10")
        };

        if let Some(block_hash) = get_hash_from_db()? {
            Ok(Some((block_n, block_hash)))
        } else {
            // only subscribe when block is not found
            loop {
                let mut receiver = self.backend.subscribe_closed_blocks();
                if let Some(block_hash) = get_hash_from_db()? {
                    break Ok(Some((block_n, block_hash)));
                }
                match receiver.blocking_recv() {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        anyhow::bail!("Backend latest block channel closed")
                    }
                }
            }
        }
    }

    fn new_block(&mut self) -> anyhow::Result<()> {
        let mut cached_state = self.executor.block_state.take().expect("Executor block state alreadt taken");

        let state = cached_state.to_state_diff().context("Cannot make state diff")?.state_maps;
        let declared_classes: HashMap<_, _> = mem::take(&mut self.declared_classes)
            .into_iter()
            .map(|class| {
                let class_hash = class.class_hash();
                (&class)
                    .try_into()
                    .map_err(|e| anyhow::anyhow!("Cannot convert to blockifier runnable class: {e}"))
                    .map(|c: RunnableCompiledClass| (ClassHash(class_hash), c))
            })
            .collect::<Result<_, _>>()?;

        let block_n = self.on_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);

        let mut adaptor = cached_state.state;

        adaptor.inner =
            BlockifierStateAdapter::new(Arc::clone(&self.backend), block_n + 1, Some(DbBlockId::Number(block_n)));

        adaptor.push_to_cache(block_n, state, declared_classes);
        if let Some(block_n) = self.backend.get_latest_block_n().context("Getting latest block_n")? {
            adaptor.remove_cache_older_than(block_n);
        }

        let (blockifier_executor, new_exec_ctx) =
            create_blockifier_executor(adaptor, &self.backend, &self.l1_data_provider, self.on_block_n)?;
        self.on_block_n = Some(block_n);
        self.executor = blockifier_executor;
        self.new_exec_ctx = Some(new_exec_ctx);
        Ok(())
    }

    fn run(mut self) -> anyhow::Result<()> {
        let mut block_empty = true;
        let mut block_now_full = false;

        let batch_size = self.backend.chain_config().execution_batch_size;
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        let mut txs_to_process = VecDeque::with_capacity(batch_size);
        let mut txs_to_process_blockifier = Vec::with_capacity(batch_size);

        let mut next_block_deadline = Instant::now() + block_time;

        // Cloning transactions: That's a lot of cloning, but we're kind of forced to do that because blockifier takes
        // a `&[Transaction]` slice. In addition, declare transactions have their class behind an Arc.
        loop {
            let mut reply = BatchReply::default();

            let start_time = Instant::now();
            let block_time_deadline_reached = next_block_deadline >= start_time;
            // bouncer cap reached last batch, or block time deadline reached
            // (but, don't emit empty blocks if the config says so)
            if block_now_full || (block_time_deadline_reached && !(block_empty && no_empty_blocks)) {
                // Block needs to be closed.
                self.new_block().context("Preparing new block execution state")?;
                next_block_deadline = start_time + block_time;
                block_empty = true;
                block_now_full = false;
            }

            if self.new_exec_ctx.is_some() {
                // We are making a new block - we need to put the hash of current_block_n-10 into the state diff.
                // current_block_n-10 however might not be saved into the database yet. In that case, we have to wait.
                // This shouldn't create a deadlock (cyclic wait) unless the database is in a weird state (?)

                // https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1
                if let Some((block_n_min_10, block_hash_n_min_10)) = self.wait_for_hash_of_block_min_10()? {
                    let contract_address = ContractAddress(PatriciaKey::ONE);
                    let key =
                        StorageKey::try_from(Felt::from(block_n_min_10)).context("Block_n overflows storage key")?;
                    self.executor
                        .block_state
                        .as_mut()
                        .expect("Blockifier block context has been taken")
                        .set_storage_at(contract_address, key, block_hash_n_min_10)
                        .context("Cannot set storage value in cache")?;
                    reply.new_state_diffs.storage.insert((contract_address, key), block_hash_n_min_10);
                }
            }

            // Take transactions to execute.
            if txs_to_process.len() < batch_size {
                let Some(taken) = self.wait_take_tx_batch(next_block_deadline) else {
                    return Ok(()); // Channel closed. Exit gracefully.
                };

                reply.stats.n_taken += taken.len();
                txs_to_process_blockifier.extend(txs_to_process.iter().map(|tx: &MempoolTransaction| tx.tx.clone()));
                txs_to_process.extend(taken);
            }

            reply.stats.n_batches += 1;

            // Execute the transactions.
            let all_results = self.executor.execute_txs(&txs_to_process_blockifier);
            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            block_now_full = all_results.len() < txs_to_process_blockifier.len();

            txs_to_process_blockifier.drain(..all_results.len()); // remove the used txs

            for exec_result in all_results {
                let mut mempool_tx = txs_to_process.pop_front().context("Vector length mismatch")?;

                reply.executed.push(mempool_tx.tx_hash().to_felt());

                match exec_result {
                    Ok((execution_info, state_diff)) => {
                        // Reverted transactions appear here as Ok too.
                        tracing::trace!("Successful execution of transaction {:#x}", mempool_tx.tx_hash().to_felt());

                        block_empty = false;

                        reply.stats.n_added_to_block += 1;
                        if execution_info.is_reverted() {
                            reply.stats.n_reverted += 1;
                        }

                        let receipt = from_blockifier_execution_info(&execution_info, &mempool_tx.tx);
                        let converted_tx = TransactionWithHash::from(mempool_tx.tx.clone());

                        if let Some(class) = mempool_tx.converted_class.take() {
                            reply.new_declared_classes.push(class.clone());
                            self.declared_classes.push(class);
                        }
                        reply.new_events.extend(
                            receipt
                                .events()
                                .iter()
                                .cloned()
                                .map(|event| EventWithTransactionHash { event, transaction_hash: converted_tx.hash }),
                        );
                        reply.new_state_diffs.extend(&state_diff);
                        reply
                            .new_transactions
                            .push(TransactionWithReceipt { transaction: converted_tx.transaction, receipt });
                    }
                    Err(err) => {
                        // These are the transactions that have errored but we can't revert them. It can be because of an internal server error, but
                        // errors during the execution of Declare and DeployAccount also appear here as they cannot be reverted.
                        // We reject them.
                        // Note that this is a big DoS vector.
                        tracing::error!(
                            "Rejected transaction {:#x} for unexpected error: {err:#}",
                            mempool_tx.tx_hash().to_felt()
                        );
                        reply.stats.n_rejected += 1;
                    }
                }
            }

            reply.stats.n_excess = txs_to_process.len();

            reply.new_execution_context = self.new_exec_ctx.take();
            reply.stats.exec_duration += start_time.elapsed();

            tracing::debug!("Weights: {:?}", self.executor.bouncer.get_accumulated_weights());
            tracing::debug!("Stats: {:?}", reply.stats);
            tracing::debug!("Block now full: {:?}", block_now_full);

            // will block if consumer takes too much time (block closing, database etc.)
            if self.replies_sender.blocking_send(reply).is_err() {
                // Receiver closed
                break Ok(());
            }
        }
    }
}
