//! Block production service.

use crate::metrics::BlockProductionMetrics;
use anyhow::Context;
use blockifier::state::cached_state::{StateMaps, StorageEntry};
use blockifier::transaction::transaction_execution::Transaction;
use executor::{AdditionalTxInfo, BatchExecutionResult, BatchToExecute, Executor, ExecutorMessage};
use mc_db::db_block_id::DbBlockId;
use mc_db::MadaraBackend;
use mc_mempool::{L1DataProvider, Mempool, MempoolProvider};
use mp_block::header::PendingHeader;
use mp_block::{BlockId, BlockTag, PendingFullBlock, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::{from_blockifier_execution_info, EventWithTransactionHash};
use mp_state_update::DeclaredClassItem;
use mp_transactions::TransactionWithHash;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use opentelemetry::KeyValue;
use starknet_types_core::felt::Felt;
use std::collections::HashMap;
use std::mem;
use std::ops::{Add, AddAssign};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use util::{state_map_to_state_diff, BlockExecutionContext};

mod executor;
pub mod metrics;
mod util;

// TODO: add this to metrics
#[derive(Default, Clone, Debug)]
pub struct ContinueBlockStats {
    /// Number of batches executed before reaching the bouncer capacity.
    pub n_batches: usize,
    /// Number of transactions included into the block.
    pub n_added_to_block: usize,
    /// Number of transactions executed.
    pub n_executed: usize,
    /// Reverted transactions are failing transactions that are included in the block.
    pub n_reverted: usize,
    /// Rejected are txs are failing transactions that are not revertible. They are thus not included in the block
    pub n_rejected: usize,
    /// Number of declared classes.
    pub declared_classes: usize,
    /// Execution time
    pub exec_duration: Duration,
}

impl Add for ContinueBlockStats {
    type Output = Self;
    fn add(self, other: Self) -> Self::Output {
        Self {
            n_batches: self.n_batches + other.n_batches,
            n_added_to_block: self.n_added_to_block + other.n_added_to_block,
            n_executed: self.n_executed + other.n_executed,
            n_reverted: self.n_reverted + other.n_reverted,
            n_rejected: self.n_rejected + other.n_rejected,
            declared_classes: self.declared_classes + other.declared_classes,
            exec_duration: self.exec_duration + other.exec_duration,
        }
    }
}
impl AddAssign for ContinueBlockStats {
    fn add_assign(&mut self, rhs: Self) {
        *self = self.clone() + rhs
    }
}

#[derive(Debug, Clone)]
struct PendingBlockState {
    pub header: PendingHeader,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
    pub declared_classes: Vec<ConvertedClass>,
    pub state_diff: StateMaps,
}

impl PendingBlockState {
    pub fn new_from_execution_context(
        exec_ctx: BlockExecutionContext,
        parent_block_hash: Felt,
        initial_state_diffs_storage: HashMap<StorageEntry, Felt>,
    ) -> Self {
        Self::new(exec_ctx.into_header(parent_block_hash), initial_state_diffs_storage)
    }

    pub fn new(header: PendingHeader, initial_state_diffs_storage: HashMap<StorageEntry, Felt>) -> Self {
        Self {
            header,
            transactions: Default::default(),
            events: Default::default(),
            declared_classes: Default::default(),
            state_diff: StateMaps {
                nonces: Default::default(),
                class_hashes: Default::default(),
                storage: initial_state_diffs_storage,
                compiled_class_hashes: Default::default(),
                declared_contracts: Default::default(),
            },
        }
    }

    pub fn into_full_block_with_classes(
        self,
        backend: &MadaraBackend,
        block_n: u64,
    ) -> anyhow::Result<(PendingFullBlock, Vec<ConvertedClass>)> {
        let on_top_of_block_id = block_n.checked_sub(1).map(DbBlockId::Number);
        Ok((
            PendingFullBlock {
                header: self.header,
                state_diff: state_map_to_state_diff(backend, &on_top_of_block_id, self.state_diff)
                    .context("Converting state map to state diff")?,
                transactions: self.transactions,
                events: self.events,
            },
            self.declared_classes,
        ))
    }
}

// TODO: move to backend
pub fn get_pending_block_from_db(backend: &MadaraBackend) -> anyhow::Result<(PendingFullBlock, Vec<ConvertedClass>)> {
    let pending_block = backend
        .get_block(&DbBlockId::Pending)
        .context("Getting pending block")?
        .context("No pending block")?
        .into_pending()
        .context("Block is not pending")?;
    let state_diff = backend.get_pending_block_state_update().context("Getting pending state update")?;

    let classes: Vec<_> = state_diff
        .deprecated_declared_classes
        .iter()
        .chain(state_diff.declared_classes.iter().map(|DeclaredClassItem { class_hash, .. }| class_hash))
        .map(|class_hash| {
            backend
                .get_converted_class(&BlockId::Tag(BlockTag::Pending), class_hash)
                .with_context(|| format!("Retrieving pending declared class with hash {class_hash:#x}"))?
                .with_context(|| format!("Pending declared class with hash {class_hash:#x} not found"))
        })
        .collect::<Result<_, _>>()?;

    // Close and import the pending block
    let block = PendingFullBlock {
        header: pending_block.info.header,
        events: pending_block.inner.events().collect(),
        transactions: pending_block
            .inner
            .transactions
            .into_iter()
            .zip(pending_block.inner.receipts)
            .map(|(transaction, receipt)| TransactionWithReceipt { transaction, receipt })
            .collect(),
        state_diff,
    };

    Ok((block, classes))
}

#[derive(Debug)]
pub(crate) struct CurrentPendingState {
    pub block: PendingBlockState,
    pub block_n: u64,
    // These are reset every pending tick.
    pub tx_executed_for_tick: Vec<Felt>,
    pub stats_for_tick: ContinueBlockStats,
}

impl CurrentPendingState {
    pub fn new(block: PendingBlockState, block_n: u64) -> Self {
        Self { block, block_n, tx_executed_for_tick: Default::default(), stats_for_tick: Default::default() }
    }
    pub fn append_batch(&mut self, batch: BatchExecutionResult) {
        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            self.tx_executed_for_tick.push(Transaction::tx_hash(&blockifier_tx).to_felt());

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                if let Some(class) = additional_info.declared_class.take() {
                    if !execution_info.is_reverted() {
                        self.block.declared_classes.push(*class);
                    }
                }

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);
                let converted_tx = TransactionWithHash::from(blockifier_tx.clone());

                self.block.events.extend(
                    receipt
                        .events()
                        .iter()
                        .cloned()
                        .map(|event| EventWithTransactionHash { event, transaction_hash: converted_tx.hash }),
                );
                self.block.state_diff.extend(&state_diff);
                self.block.transactions.push(TransactionWithReceipt { transaction: converted_tx.transaction, receipt });
            }
        }
        self.stats_for_tick += batch.stats;
    }
}

/// Used to listen to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock,
    UpdatedPendingBlock,
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
pub(crate) enum ExecutorState {
    NotExecuting {
        latest_block_n: Option<u64>,
        /// [`Felt::ZERO`] for genesis.
        /// When receiving a BlockEnd message, we'll close the block and transition to the NotExecuting state with its block hash as this field.
        /// This block hash is not known by the executor thread yet - because we're closing the block in parallel while the executor thread might be executing
        /// another new batch for the next block.
        latest_block_hash: Felt,
    },
    Executing(Box<CurrentPendingState>),
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
    l1_data_provider: Arc<dyn L1DataProvider>,
    mempool: Arc<Mempool>,
    current_state: Option<ExecutorState>,
    metrics: Arc<BlockProductionMetrics>,
    state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
}

impl BlockProductionTask {
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

    /// Closes the last pending block store in db (if any).
    ///
    /// This avoids re-executing transaction by re-adding them to the [Mempool],
    /// as was done before.
    async fn close_pending_block_if_exists(&mut self) -> anyhow::Result<()> {
        // We cannot use `backend.get_block` to check for the existence of the
        // pending block as it will ALWAYS return a pending block, even if there
        // is none in db (it uses the Default::default in that case).
        if !self.backend.has_pending_block().context("Error checking if pending block exists")? {
            return Ok(());
        }

        tracing::debug!("Close pending block on startup.");

        let (block, declared_classes) = get_pending_block_from_db(&self.backend)?;

        // NOTE: we disabled the Write Ahead Log when clearing the pending block
        // so this will be done atomically at the same time as we close the next
        // block, after we manually flush the db.
        self.backend.clear_pending_block().context("Error clearing pending block")?;

        let block_n = self.backend.get_latest_block_n().context("Getting latest block n")?.map(|n| n + 1).unwrap_or(0);
        self.close_and_save_block(block_n, block, declared_classes, vec![]).await?;

        Ok(())
    }

    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> Self {
        Self { backend, l1_data_provider, mempool, current_state: None, metrics, state_notifications: None }
    }

    /// Returns the block_hash.
    async fn close_and_save_block(
        &mut self,
        block_n: u64,
        block: PendingFullBlock,
        classes: Vec<ConvertedClass>,
        tx_executed: Vec<Felt>,
    ) -> anyhow::Result<Felt> {
        tracing::debug!("Close and save block block_n={block_n}");
        let start_time = Instant::now();

        let n_txs = block.transactions.len();

        // Close and import the block
        let block_hash = self
            .backend
            .add_full_block_with_classes(block, block_n, &classes, /* pre_v0_13_2_hash_override */ true)
            .await
            .context("Error closing block")?;

        self.mempool.remove_txs(tx_executed).context("Removing mempool transactions")?;

        // // Flush changes to disk
        // self.backend.flush().map_err(|err| Error::Unexpected(format!("DB flushing error: {err:#}").into()))?;

        let end_time = start_time.elapsed();
        tracing::info!("⛏️  Closed block #{} with {} transactions - {:?}", block_n, n_txs, end_time);

        // Record metrics
        let attributes = [
            KeyValue::new("transactions_added", n_txs.to_string()),
            KeyValue::new("closing_time", end_time.as_secs_f32().to_string()),
        ];

        self.metrics.block_counter.add(1, &[]);
        self.metrics.block_gauge.record(block_n, &attributes);
        self.metrics.transaction_counter.add(n_txs as u64, &[]);

        self.send_state_notification(BlockProductionStateNotification::ClosedBlock);

        Ok(block_hash)
    }

    /// Handles the state machine and its transitions.
    async fn process_reply(&mut self, reply: ExecutorMessage) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { initial_state_diffs_storage, exec_ctx } => {
                tracing::debug!("Got ExecutorMessage::StartNewBlock block_n={}", exec_ctx.block_n);
                let current_state = self.current_state.take().context("No current state")?;
                let ExecutorState::NotExecuting { latest_block_n, latest_block_hash } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let new_block_n = latest_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                if new_block_n != exec_ctx.block_n {
                    anyhow::bail!(
                        "Got new block_n={} from executor, expected block_n={}",
                        exec_ctx.block_n,
                        new_block_n
                    )
                }

                self.current_state = Some(ExecutorState::Executing(
                    CurrentPendingState::new(
                        PendingBlockState::new_from_execution_context(
                            exec_ctx,
                            latest_block_hash,
                            initial_state_diffs_storage,
                        ),
                        new_block_n,
                    )
                    .into(),
                ));
            }
            ExecutorMessage::BatchExecuted(batch_execution_result) => {
                tracing::debug!(
                    "Got ExecutorMessage::BatchExecuted executed_txs={}",
                    batch_execution_result.executed_txs.len()
                );
                let current_state = self.current_state.as_mut().context("No current state")?;
                let ExecutorState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                state.append_batch(batch_execution_result);
            }
            ExecutorMessage::EndBlock => {
                tracing::debug!("Got ExecutorMessage::EndBlock");
                let current_state = self.current_state.take().context("No current state")?;
                let ExecutorState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                let (block, classes) = state.block.into_full_block_with_classes(&self.backend, state.block_n)?;
                let block_hash = self
                    .close_and_save_block(state.block_n, block, classes, state.tx_executed_for_tick)
                    .await
                    .context("Closing and saving block")?;

                self.current_state = Some(ExecutorState::NotExecuting {
                    latest_block_n: Some(state.block_n),
                    latest_block_hash: block_hash,
                });
            }
        }

        Ok(())
    }

    fn store_pending_block(&mut self) -> anyhow::Result<()> {
        if let ExecutorState::Executing(state) = self.current_state.as_mut().context("No current state")? {
            self.mempool
                .remove_txs(mem::take(&mut state.tx_executed_for_tick))
                .context("Removing mempool transactions")?;

            let (block, classes) = state
                .block
                .clone()
                .into_full_block_with_classes(&self.backend, state.block_n)
                .context("Converting to full pending block")?;
            self.backend.store_pending_block_with_classes(block, &classes)?;

            // self.backend.flush().context("DB flushing")?;

            let stats = mem::take(&mut state.stats_for_tick);
            if stats.n_added_to_block > 0 {
                tracing::info!(
                    "🧮 Executed and added {} transaction(s) to the pending block at height {} - {:.3?}",
                    stats.n_added_to_block,
                    state.block_n,
                    stats.exec_duration,
                );
                tracing::debug!("Tick stats {:?}", stats);
            }
        }

        self.send_state_notification(BlockProductionStateNotification::UpdatedPendingBlock);

        Ok(())
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, mut ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        if let Err(err) = self.close_pending_block_if_exists().await {
            // This error should not stop block production from working. If it happens, that's too bad. We drop the pending state and start from
            // a fresh one.
            tracing::error!("Failed to continue the pending block state: {err:#}");
        }

        let batch_size = self.backend.chain_config().execution_batch_size;

        // initial state
        let latest_block_n = self.backend.get_latest_block_n().context("Getting latest block_n")?;
        let latest_block_hash = if let Some(block_n) = latest_block_n {
            self.backend
                .get_block_hash(&DbBlockId::Number(block_n))
                .context("Getting latest block hash")?
                .context("Block not found")?
        } else {
            Felt::ZERO // Genesis' parent block hash.
        };
        self.current_state = Some(ExecutorState::NotExecuting { latest_block_n, latest_block_hash });

        let mut executor = Executor::create(Arc::clone(&self.backend), Arc::clone(&self.l1_data_provider))
            .context("Starting executor")?;

        let mut interval_pending_block_update =
            tokio::time::interval(self.backend.chain_config().pending_block_update_time);
        interval_pending_block_update.reset(); // Skip the immediate first tick.
        interval_pending_block_update.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // Separate batch maker from our processing of the replies.
        let mempool = Arc::clone(&self.mempool);
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let mut batch_maker_task = AbortOnDrop::spawn(async move {
            loop {
                // We use the permit API so that we don't have to remove transactions from the mempool until the last moment.
                // The buffer inside of the channel is of size 1 - meaning we're preparing the next batch of transactions that will immediately be executed next, once
                // the worker has finished executing its current one.
                let Some(Ok(permit)) = ctx.run_until_cancelled(batch_sender.reserve()).await else {
                    // Stop condition: service stopped (ctx), or batch sender closed.
                    return anyhow::Ok(());
                };
                let mut batch = BatchToExecute::with_capacity(batch_size);
                let Some(iterator) = ctx.run_until_cancelled(mempool.get_popping_iterator_or_wait()).await else {
                    // Stop condition: service stopped (ctx).
                    return anyhow::Ok(());
                };

                // FIXME: add this to the debug logging just below. (the number is wrong right now (?))
                // let n_txs_in_mempool = iterator.n_txs_total();

                let iterator = iterator.take(batch_size); // only take a batch

                for tx in iterator {
                    let additional = AdditionalTxInfo { declared_class: tx.converted_class };
                    batch.push(tx.tx, additional);
                }

                tracing::debug!("Sending batch of {} transactions to the worker thread.", batch.len());

                permit.send(batch);
            }
        });

        // Graceful shutdown: when the service is asked to stop, the `batch_maker_task` will stop,
        //  which will close the `send_batch` channel (by dropping it). The executor thread then will see that the channel
        //  is closed next time it tries to receive from it. The executor thread shuts down, dropping the `executor.stop` channel,
        //  therefore closing it as well.
        // We will then see the anyhow::Ok(()) result in the stop channel, as per the implementation of [`StopErrorReceiver::recv`].
        // Note that for this to work, we need to make sure the `send_batch` channel is never aliased -
        //  otherwise it will never not be closed automatically.

        loop {
            tokio::select! {
                // Bubble up errors from the executor thread, or graceful shutdown.
                res = executor.stop.recv() => return res.context("In executor thread"),

                // Bubble up errors from the batch maker task. (tokio JoinHandle)
                res = &mut batch_maker_task => return res.context("In batch maker task"),

                // Process results from the execution
                Some(reply) = executor.replies.recv() => {
                    self.process_reply(reply).await.context("Processing reply from executor thread")?;
                }

                // Update the pending block in db periodically.
                _ = interval_pending_block_update.tick() => {
                    self.store_pending_block().context("Storing pending block")?;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::BlockProductionStateNotification;
    use crate::{metrics::BlockProductionMetrics, util::state_map_to_state_diff, BlockProductionTask};
    use blockifier::transaction::transaction_execution::Transaction as BTransaction;
    use blockifier::{
        bouncer::{BouncerConfig, BouncerWeights},
        state::cached_state::StateMaps,
    };
    use mc_db::{db_block_id::DbBlockId, MadaraBackend};
    use mc_devnet::{Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector};
    use mc_mempool::{Mempool, MempoolConfig, MockL1DataProvider};
    use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
    use mp_block::header::{GasPrices, L1DataAvailabilityMode};
    use mp_chain_config::ChainConfig;
    use mp_convert::{felt, ToFelt};
    use mp_rpc::{
        BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedInvokeTxn, BroadcastedTxn, DaMode, InvokeTxnV3,
        ResourceBounds, ResourceBoundsMapping,
    };
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
        StorageEntry,
    };
    use mp_transactions::{BroadcastedTransactionExt, Transaction};
    use mp_utils::service::ServiceContext;
    use mp_utils::AbortOnDrop;
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
    use starknet_types_core::felt::Felt;
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tracing_test::traced_test;

    // TODO(block_production_tests): These tests are very lacking, and a lot of them rely a lot on implementation details that should not be tested for.
    // Here is a short list of tests that should ideally be here, instead of these ones:
    // - [ ] Test different transaction types, including L1HandlerTransaction.
    // - [ ] Test that earlier state changes are taken into account in the current block -> block 2 should see state changes from block 1.
    //  => especially relevant now that we are executing stuff on another thread, and the state may be not yet saved into the database.
    // - [x] Test that new transactions roll over to a new block on block_time.
    // - [x] Test that new transactions roll over to a new block on bouncer config.
    // - [ ] Test that the pending block is updated correctly, including its state diff, receipts and events.
    // - [ ] Test that blocks are able to be closed correctly, including its state diff, receipts and events AND global state changes.
    // - [ ] Test that state diffs are produced with contract 0x1 containing the block hash of block_n - 10.
    // - [ ] Test that in case of a graceful stop, we don't lose transactions nor do we lose pending blocks / full blocks / anything.
    // - [ ] Test that the no_empty_block chain config is working.
    // - [ ] Test that block time is respected even when mempool is very very full. (there used to be a bug around this)
    // - [ ] Test that mempool notify is respected (how we do that? unsure atm)
    // - [ ] Test rejected transactions are removed from the mempool.
    // - [ ] Test where you close more than a single block.
    // - [ ] Test that an empty block can be produced when reaching the block_time.
    //
    // In addition, we should not rely on timeouts in our tests. Instead, use a test-only channel with notifications about the current state of block production.
    // This has been partly implemented.

    type TxFixtureInfo = (Transaction, mp_receipt::TransactionReceipt);

    #[rstest::fixture]
    fn backend() -> Arc<MadaraBackend> {
        MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_devnet()))
    }

    #[rstest::fixture]
    fn bouncer_weights() -> BouncerWeights {
        // The bouncer weights values are configured in such a way
        // that when loaded, the block will close after one transaction
        // is added to it, to test the pending tick closing the block
        BouncerWeights {
            sierra_gas: starknet_api::execution_resources::GasAmount(10000000),
            message_segment_length: 10000,
            n_events: 10000,
            state_diff_size: 10000,
            l1_gas: 10000,
        }
    }

    #[rstest::fixture]
    fn setup(backend: Arc<MadaraBackend>) -> (Arc<MadaraBackend>, Arc<BlockProductionMetrics>) {
        (Arc::clone(&backend), Arc::new(BlockProductionMetrics::register()))
    }

    #[rstest::fixture]
    async fn devnet_setup(
        #[default(16)] execution_batch_size: usize,
        #[default(Duration::from_secs(30))] block_time: Duration,
        #[default(Duration::from_secs(2))] pending_block_update_time: Duration,
        #[default(false)] use_bouncer_weights: bool,
    ) -> (
        Arc<MadaraBackend>,
        Arc<BlockProductionMetrics>,
        Arc<MockL1DataProvider>,
        Arc<Mempool>,
        Arc<TransactionValidator>,
        DevnetKeys,
    ) {
        let mut genesis = ChainGenesisDescription::base_config().unwrap();
        let contracts = genesis.add_devnet_contracts(10).unwrap();

        let chain_config: Arc<ChainConfig> = if use_bouncer_weights {
            let bouncer_weights = bouncer_weights();

            Arc::new(ChainConfig {
                execution_batch_size,
                block_time,
                pending_block_update_time,
                bouncer_config: BouncerConfig { block_max_capacity: bouncer_weights },
                ..ChainConfig::madara_devnet()
            })
        } else {
            Arc::new(ChainConfig {
                execution_batch_size,
                block_time,
                pending_block_update_time,
                ..ChainConfig::madara_devnet()
            })
        };

        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        genesis.build_and_store(&backend).await.unwrap();

        let mut l1_data_provider = MockL1DataProvider::new();
        l1_data_provider.expect_get_da_mode().return_const(L1DataAvailabilityMode::Blob);
        l1_data_provider.expect_get_gas_prices().return_const(GasPrices {
            eth_l1_gas_price: 128,
            strk_l1_gas_price: 128,
            eth_l1_data_gas_price: 128,
            strk_l1_data_gas_price: 128,
        });
        let l1_data_provider = Arc::new(l1_data_provider);

        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::for_testing()));
        let tx_validator = Arc::new(TransactionValidator::new(
            Arc::clone(&mempool) as _,
            Arc::clone(&backend),
            TransactionValidatorConfig::default(),
        ));

        (
            backend,
            Arc::new(BlockProductionMetrics::register()),
            Arc::clone(&l1_data_provider),
            mempool,
            tx_validator,
            contracts,
        )
    }

    #[rstest::fixture]
    fn tx_invoke_v0(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
        (
            mp_transactions::Transaction::Invoke(mp_transactions::InvokeTransaction::V0(
                mp_transactions::InvokeTransactionV0 { contract_address, ..Default::default() },
            )),
            mp_receipt::TransactionReceipt::Invoke(mp_receipt::InvokeTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    fn tx_l1_handler(#[default(Felt::ZERO)] contract_address: Felt) -> TxFixtureInfo {
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
    fn tx_deploy() -> TxFixtureInfo {
        (
            mp_transactions::Transaction::Deploy(mp_transactions::DeployTransaction::default()),
            mp_receipt::TransactionReceipt::Deploy(mp_receipt::DeployTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    fn tx_deploy_account() -> TxFixtureInfo {
        (
            mp_transactions::Transaction::DeployAccount(mp_transactions::DeployAccountTransaction::V1(
                mp_transactions::DeployAccountTransactionV1::default(),
            )),
            mp_receipt::TransactionReceipt::DeployAccount(mp_receipt::DeployAccountTransactionReceipt::default()),
        )
    }

    #[rstest::fixture]
    fn converted_class_legacy(#[default(Felt::ZERO)] class_hash: Felt) -> mp_class::ConvertedClass {
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
    fn converted_class_sierra(
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
                compiled_class_hash,
            },
            compiled: Arc::new(mp_class::CompiledSierra("".to_string())),
        })
    }

    async fn sign_and_add_declare_tx(
        contract: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        validator: &Arc<TransactionValidator>,
        nonce: Felt,
    ) {
        let sierra_class: starknet_core::types::contract::SierraClass =
            serde_json::from_slice(m_cairo_test_contracts::TEST_CONTRACT_SIERRA).unwrap();
        let flattened_class: mp_class::FlattenedSierraClass = sierra_class.clone().flatten().unwrap().into();

        // starkli class-hash target/dev/madara_contracts_TestContract.compiled_contract_class.json
        let compiled_contract_class_hash =
            Felt::from_hex("0x0138105ded3d2e4ea1939a0bc106fb80fd8774c9eb89c1890d4aeac88e6a1b27").unwrap();

        let mut declare_txn: BroadcastedDeclareTxn = BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
            sender_address: contract.address,
            compiled_class_hash: compiled_contract_class_hash,
            // this field will be filled below
            signature: vec![],
            nonce,
            contract_class: flattened_class.into(),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 210000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let (blockifier_tx, _class) = BroadcastedTxn::Declare(declare_txn.clone())
            .into_blockifier(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
                /* validate */ true,
                /* charge_fee */ true,
            )
            .unwrap();
        let signature = contract.secret.sign(&BTransaction::tx_hash(&blockifier_tx).0).unwrap();

        let tx_signature = match &mut declare_txn {
            BroadcastedDeclareTxn::V1(tx) => &mut tx.signature,
            BroadcastedDeclareTxn::V2(tx) => &mut tx.signature,
            BroadcastedDeclareTxn::V3(tx) => &mut tx.signature,
            _ => unreachable!("the declare tx is not query only"),
        };
        *tx_signature = vec![signature.r, signature.s];

        validator.submit_declare_transaction(declare_txn).await.expect("Should accept the transaction");
    }

    async fn sign_and_add_invoke_tx(
        contract_sender: &DevnetPredeployedContract,
        contract_receiver: &DevnetPredeployedContract,
        backend: &Arc<MadaraBackend>,
        validator: &Arc<TransactionValidator>,
        nonce: Felt,
    ) {
        let erc20_contract_address =
            Felt::from_hex_unchecked("0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d");

        let mut invoke_txn: BroadcastedInvokeTxn = BroadcastedInvokeTxn::V3(InvokeTxnV3 {
            sender_address: contract_sender.address,
            calldata: Multicall::default()
                .with(Call {
                    to: erc20_contract_address,
                    selector: Selector::from("transfer"),
                    calldata: vec![
                        contract_receiver.address,
                        (9_999u128 * 1_000_000_000_000_000_000).into(),
                        Felt::ZERO,
                    ],
                })
                .flatten()
                .collect(),
            // this field will be filled below
            signature: vec![],
            nonce,
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
            },
            tip: 0,
            paymaster_data: vec![],
            account_deployment_data: vec![],
            nonce_data_availability_mode: DaMode::L1,
            fee_data_availability_mode: DaMode::L1,
        });

        let (blockifier_tx, _classes) = BroadcastedTxn::Invoke(invoke_txn.clone())
            .into_blockifier(
                backend.chain_config().chain_id.to_felt(),
                backend.chain_config().latest_protocol_version,
                /* validate */ true,
                /* charge_fee */ true,
            )
            .unwrap();
        let signature = contract_sender.secret.sign(&BTransaction::tx_hash(&blockifier_tx)).unwrap();

        let tx_signature = match &mut invoke_txn {
            BroadcastedInvokeTxn::V0(tx) => &mut tx.signature,
            BroadcastedInvokeTxn::V1(tx) => &mut tx.signature,
            BroadcastedInvokeTxn::V3(tx) => &mut tx.signature,
            _ => unreachable!("the invoke tx is not query only"),
        };
        *tx_signature = vec![signature.r, signature.s];

        validator.submit_invoke_transaction(invoke_txn).await.expect("Should accept the transaction");
    }

    #[rstest::rstest]
    fn test_block_prod_state_map_to_state_diff(backend: Arc<MadaraBackend>) {
        let mut nonces = HashMap::new();
        nonces.insert(felt!("1").try_into().unwrap(), Nonce(felt!("1")));
        nonces.insert(felt!("2").try_into().unwrap(), Nonce(felt!("2")));
        nonces.insert(felt!("3").try_into().unwrap(), Nonce(felt!("3")));

        let mut class_hashes = HashMap::new();
        class_hashes.insert(felt!("1").try_into().unwrap(), ClassHash(felt!("0xc1a551")));
        class_hashes.insert(felt!("2").try_into().unwrap(), ClassHash(felt!("0xc1a552")));
        class_hashes.insert(felt!("3").try_into().unwrap(), ClassHash(felt!("0xc1a553")));

        let mut storage = HashMap::new();
        storage.insert((felt!("1").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("1").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("1").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        storage.insert((felt!("2").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("2").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("2").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        storage.insert((felt!("3").try_into().unwrap(), felt!("1").try_into().unwrap()), felt!("1"));
        storage.insert((felt!("3").try_into().unwrap(), felt!("2").try_into().unwrap()), felt!("2"));
        storage.insert((felt!("3").try_into().unwrap(), felt!("3").try_into().unwrap()), felt!("3"));

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(ClassHash(felt!("0xc1a551")), CompiledClassHash(felt!("0x1")));
        compiled_class_hashes.insert(ClassHash(felt!("0xc1a552")), CompiledClassHash(felt!("0x2")));

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(ClassHash(felt!("0xc1a551")), true);
        declared_contracts.insert(ClassHash(felt!("0xc1a552")), true);
        declared_contracts.insert(ClassHash(felt!("0xc1a553")), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: felt!("1"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!("2"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!("3"),
                storage_entries: vec![
                    StorageEntry { key: felt!("1"), value: Felt::ONE },
                    StorageEntry { key: felt!("2"), value: Felt::TWO },
                    StorageEntry { key: felt!("3"), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![felt!("0xc1a553")];

        let declared_classes = vec![
            DeclaredClassItem { class_hash: felt!("0xc1a551"), compiled_class_hash: felt!("0x1") },
            DeclaredClassItem { class_hash: felt!("0xc1a552"), compiled_class_hash: felt!("0x2") },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: felt!("1"), nonce: felt!("1") },
            NonceUpdate { contract_address: felt!("2"), nonce: felt!("2") },
            NonceUpdate { contract_address: felt!("3"), nonce: felt!("3") },
        ];

        let deployed_contracts = vec![
            DeployedContractItem { address: felt!("1"), class_hash: felt!("0xc1a551") },
            DeployedContractItem { address: felt!("2"), class_hash: felt!("0xc1a552") },
            DeployedContractItem { address: felt!("3"), class_hash: felt!("0xc1a553") },
        ];

        let replaced_classes = vec![];

        let expected = StateDiff {
            storage_diffs,
            deprecated_declared_classes,
            declared_classes,
            nonces,
            deployed_contracts,
            replaced_classes,
        };

        let mut actual = state_map_to_state_diff(&backend, &Option::<_>::None, state_map).unwrap();

        actual.storage_diffs.sort_by(|a, b| a.address.cmp(&b.address));
        actual.storage_diffs.iter_mut().for_each(|s| s.storage_entries.sort_by(|a, b| a.key.cmp(&b.key)));
        actual.deprecated_declared_classes.sort();
        actual.declared_classes.sort_by(|a, b| a.class_hash.cmp(&b.class_hash));
        actual.nonces.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
        actual.deployed_contracts.sort_by(|a, b| a.address.cmp(&b.address));
        actual.replaced_classes.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));

        assert_eq!(
            actual,
            expected,
            "actual: {}\nexpected: {}",
            serde_json::to_string_pretty(&actual).unwrap_or_default(),
            serde_json::to_string_pretty(&expected).unwrap_or_default()
        );
    }

    /// This test makes sure that if a pending block is already present in db
    /// at startup, then it is closed and stored in db.
    ///
    /// This happens if a full node is shutdown (gracefully or not) midway
    /// during block production.
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_pass(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
        #[from(converted_class_legacy)]
        #[with(Felt::ZERO)]
        converted_class_legacy_0: mp_class::ConvertedClass,
        #[from(converted_class_sierra)]
        #[with(Felt::ONE, Felt::ONE)]
        converted_class_sierra_1: mp_class::ConvertedClass,
        #[from(converted_class_sierra)]
        #[with(Felt::TWO, Felt::TWO)]
        converted_class_sierra_2: mp_class::ConvertedClass,
    ) {
        let (backend, metrics, l1_data_provider, mempool, _tx_validator, _contracts) = devnet_setup.await;

        // ================================================================== //
        //                  PART 1: we prepare the pending block              //
        // ================================================================== //

        let pending_inner = mp_block::MadaraBlockInner {
            transactions: vec![tx_invoke_v0.0, tx_l1_handler.0, tx_declare_v0.0, tx_deploy.0, tx_deploy_account.0],
            receipts: vec![tx_invoke_v0.1, tx_l1_handler.1, tx_declare_v0.1, tx_deploy.1, tx_deploy_account.1],
        };

        let pending_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            deprecated_declared_classes: vec![Felt::ZERO],
            declared_classes: vec![
                DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE },
                DeclaredClassItem { class_hash: Felt::TWO, compiled_class_hash: Felt::TWO },
            ],
            deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
            replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let converted_classes =
            vec![converted_class_legacy_0.clone(), converted_class_sierra_1.clone(), converted_class_sierra_2.clone()];

        // ================================================================== //
        //                   PART 2: storing the pending block                //
        // ================================================================== //

        // This simulates a node restart after shutting down midway during block
        // production.
        //
        // Block production functions by storing un-finalized blocks as pending.
        // This is the only form of data we can recover without re-execution as
        // everything else is stored in RAM (mempool transactions which have not
        // been polled yet are also stored in db for retrieval, but these
        // haven't been executed anyways). This means that if ever the node
        // crashes, we will only be able to retrieve whatever data was stored in
        // the pending block. This is done atomically so we never commit partial
        // data to the database and only a full pending block can ever be
        // stored.
        //
        // We are therefore simulating stopping and restarting the node, since:
        //
        // - This is the only pending data that can persist a node restart, and
        //   it cannot be partially valid (we still test failing cases though).
        //
        // - Upon restart, this is what the block production would be looking to
        //   seal.

        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock {
                    info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
                        header: mp_block::header::PendingHeader::default(),
                        tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
                    }),
                    inner: pending_inner.clone(),
                },
                pending_state_diff.clone(),
                converted_classes.clone(),
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //        PART 3: init block production and seal pending block        //
        // ================================================================== //

        // This should load the pending block from db and close it
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check this was the case.
        assert_eq!(backend.get_latest_block_n().unwrap(), Some(1));

        let block_inner = backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;
        assert_eq!(block_inner, pending_inner);

        let state_diff = backend
            .get_block_state_diff(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest state diff from db")
            .expect("Missing latest state diff");
        assert_eq!(state_diff, pending_state_diff);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ZERO)
            .expect("Failed to retrieve class at hash 0x0 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_legacy_0);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ONE)
            .expect("Failed to retrieve class at hash 0x1 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_1);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::TWO)
            .expect("Failed to retrieve class at hash 0x2 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_2);

        // visited segments and bouncer weights are currently not stored for in
        // ready blocks
    }

    /// This test makes sure that if a pending block is already present in db
    /// at startup, then it is closed and stored in db on top of the latest
    /// block.
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_pass_on_top(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),

        // Transactions
        #[from(tx_invoke_v0)]
        #[with(Felt::ZERO)]
        tx_invoke_v0_0: TxFixtureInfo,
        #[from(tx_invoke_v0)]
        #[with(Felt::ONE)]
        tx_invoke_v0_1: TxFixtureInfo,
        #[from(tx_l1_handler)]
        #[with(Felt::ONE)]
        tx_l1_handler_1: TxFixtureInfo,
        #[from(tx_l1_handler)]
        #[with(Felt::TWO)]
        tx_l1_handler_2: TxFixtureInfo,
        #[from(tx_declare_v0)]
        #[with(Felt::TWO)]
        tx_declare_v0_2: TxFixtureInfo,
        #[from(tx_declare_v0)]
        #[with(Felt::THREE)]
        tx_declare_v0_3: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,

        // Converted classes
        #[from(converted_class_legacy)]
        #[with(Felt::ZERO)]
        converted_class_legacy_0: mp_class::ConvertedClass,
        #[from(converted_class_sierra)]
        #[with(Felt::ONE, Felt::ONE)]
        converted_class_sierra_1: mp_class::ConvertedClass,
        #[from(converted_class_sierra)]
        #[with(Felt::TWO, Felt::TWO)]
        converted_class_sierra_2: mp_class::ConvertedClass,
    ) {
        let (backend, metrics, l1_data_provider, mempool, _tx_validator, _contracts) = devnet_setup.await;

        // ================================================================== //
        //                   PART 1: we prepare the ready block               //
        // ================================================================== //

        let ready_inner = mp_block::MadaraBlockInner {
            transactions: vec![tx_invoke_v0_0.0, tx_l1_handler_1.0, tx_declare_v0_2.0],
            receipts: vec![tx_invoke_v0_0.1, tx_l1_handler_1.1, tx_declare_v0_2.1],
        };

        let ready_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
            replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let ready_converted_classes = vec![];

        // ================================================================== //
        //                   PART 2: storing the ready block                  //
        // ================================================================== //

        // Simulates block closure before the shutdown
        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock {
                    info: mp_block::MadaraMaybePendingBlockInfo::NotPending(mp_block::MadaraBlockInfo {
                        header: mp_block::Header::default(),
                        block_hash: Felt::ZERO,
                        tx_hashes: vec![Felt::ZERO, Felt::ONE, Felt::TWO],
                    }),
                    inner: ready_inner.clone(),
                },
                ready_state_diff.clone(),
                ready_converted_classes.clone(),
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //                  PART 3: we prepare the pending block              //
        // ================================================================== //

        let pending_inner = mp_block::MadaraBlockInner {
            transactions: vec![
                tx_invoke_v0_1.0,
                tx_l1_handler_2.0,
                tx_declare_v0_3.0,
                tx_deploy.0,
                tx_deploy_account.0,
            ],
            receipts: vec![tx_invoke_v0_1.1, tx_l1_handler_2.1, tx_declare_v0_3.1, tx_deploy.1, tx_deploy_account.1],
        };

        let pending_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            deprecated_declared_classes: vec![Felt::ZERO],
            declared_classes: vec![
                DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE },
                DeclaredClassItem { class_hash: Felt::TWO, compiled_class_hash: Felt::TWO },
            ],
            deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
            replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let pending_converted_classes =
            vec![converted_class_legacy_0.clone(), converted_class_sierra_1.clone(), converted_class_sierra_2.clone()];

        // ================================================================== //
        //                   PART 4: storing the pending block                //
        // ================================================================== //

        // This simulates a node restart after shutting down midway during block
        // production.
        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock {
                    info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
                        header: mp_block::header::PendingHeader::default(),
                        tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
                    }),
                    inner: pending_inner.clone(),
                },
                pending_state_diff.clone(),
                pending_converted_classes.clone(),
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //        PART 5: init block production and seal pending block        //
        // ================================================================== //

        // This should load the pending block from db and close it on top of the
        // previous block.
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check this was the case.
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);

        // Block 0 should not have been overridden!
        let block = backend
            .get_block(&mp_block::BlockId::Number(0))
            .expect("Failed to retrieve block 0 from db")
            .expect("Missing block 0");

        assert_eq!(block.info.as_closed().unwrap().header.parent_block_hash, Felt::ZERO);
        assert_eq!(block.inner, ready_inner);

        let block = backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block");

        assert_eq!(block.info.as_closed().unwrap().header.parent_block_hash, Felt::ZERO);
        assert_eq!(block.inner, pending_inner);

        // Block 0 should not have been overridden!
        let state_diff = backend
            .get_block_state_diff(&mp_block::BlockId::Number(0))
            .expect("Failed to retrieve state diff at block 0 from db")
            .expect("Missing state diff at block 0");
        assert_eq!(ready_state_diff, state_diff);

        let state_diff = backend
            .get_block_state_diff(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest state diff from db")
            .expect("Missing latest state diff");
        assert_eq!(pending_state_diff, state_diff);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ZERO)
            .expect("Failed to retrieve class at hash 0x0 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_legacy_0);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ONE)
            .expect("Failed to retrieve class at hash 0x1 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_1);

        let class = backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::TWO)
            .expect("Failed to retrieve class at hash 0x2 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_2);

        // visited segments and bouncer weights are currently not stored for in
        // ready blocks
    }

    /// This test makes sure that it is possible to start the block production
    /// task even if there is no pending block in db at the time of startup.
    #[rstest::rstest]
    #[traced_test]
    #[tokio::test]
    async fn block_prod_pending_close_on_startup_no_pending(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, _tx_validator, _contracts) = devnet_setup.await;

        // Simulates starting block production without a pending block in db
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        assert_eq!(backend.get_latest_block_n().unwrap(), Some(0)); // there is a genesis block in the db.
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check no block was added to the db
        assert_eq!(backend.get_latest_block_n().unwrap(), Some(0));
    }

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing class.
    #[rstest::rstest]
    #[traced_test]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let (backend, metrics, l1_data_provider, mempool, _tx_validator, _contracts) = devnet_setup.await;

        // ================================================================== //
        //                  PART 1: we prepare the pending block              //
        // ================================================================== //

        let pending_inner = mp_block::MadaraBlockInner {
            transactions: vec![tx_invoke_v0.0, tx_l1_handler.0, tx_declare_v0.0, tx_deploy.0, tx_deploy_account.0],
            receipts: vec![tx_invoke_v0.1, tx_l1_handler.1, tx_declare_v0.1, tx_deploy.1, tx_deploy_account.1],
        };

        let pending_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            deprecated_declared_classes: vec![],
            declared_classes: vec![DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE }],
            deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
            replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let converted_classes = vec![];

        // ================================================================== //
        //                   PART 2: storing the pending block                //
        // ================================================================== //

        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock {
                    info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
                        header: mp_block::header::PendingHeader::default(),
                        tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
                    }),
                    inner: pending_inner.clone(),
                },
                pending_state_diff.clone(),
                converted_classes.clone(),
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //        PART 3: init block production and seal pending block        //
        // ================================================================== //

        // This should fail since the pending state update references a
        // non-existent declared class at address 0x1
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        let err = block_production_task.close_pending_block_if_exists().await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing legacy class.
    #[rstest::rstest]
    #[tokio::test]
    #[traced_test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class_legacy(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let (backend, metrics, l1_data_provider, mempool, _tx_validator, _contracts) = devnet_setup.await;

        // ================================================================== //
        //                  PART 1: we prepare the pending block              //
        // ================================================================== //

        let pending_inner = mp_block::MadaraBlockInner {
            transactions: vec![tx_invoke_v0.0, tx_l1_handler.0, tx_declare_v0.0, tx_deploy.0, tx_deploy_account.0],
            receipts: vec![tx_invoke_v0.1, tx_l1_handler.1, tx_declare_v0.1, tx_deploy.1, tx_deploy_account.1],
        };

        let pending_state_diff = mp_state_update::StateDiff {
            storage_diffs: vec![
                ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::TWO,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::THREE,
                    storage_entries: vec![
                        StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
                        StorageEntry { key: Felt::ONE, value: Felt::ONE },
                        StorageEntry { key: Felt::TWO, value: Felt::TWO },
                    ],
                },
            ],
            deprecated_declared_classes: vec![Felt::ZERO],
            declared_classes: vec![],
            deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
            replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
            nonces: vec![
                NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
                NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
                NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
            ],
        };

        let converted_classes = vec![];

        // ================================================================== //
        //                   PART 2: storing the pending block                //
        // ================================================================== //

        backend
            .store_block(
                mp_block::MadaraMaybePendingBlock {
                    info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
                        header: mp_block::header::PendingHeader::default(),
                        tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
                    }),
                    inner: pending_inner.clone(),
                },
                pending_state_diff.clone(),
                converted_classes.clone(),
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //        PART 3: init block production and seal pending block        //
        // ================================================================== //

        // This should fail since the pending state update references a
        // non-existent declared class at address 0x0
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        let err = block_production_task.close_pending_block_if_exists().await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    // This test makes sure that the pending tick updates
    // the pending block correctly without closing if the bouncer limits aren't reached
    #[rstest::rstest]
    #[tokio::test]
    #[traced_test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_block_still_pending(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // Since there are no new pending blocks, this shouldn't
        // seal any blocks
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: call on pending time tick                 //
        // ================================================================== //

        // The block should still be pending since we haven't
        // reached the block limit and there should be no new
        // finalized blocks
        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert_eq!(pending_block.inner.transactions.len(), 1);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);
    }

    // // This test makes sure that the pending tick updates the correct
    // // pending block if a new pending block is added to the database
    // TODO(block_production_tests): this test does NOT make sense. Why are we testing that? This is not something we should allow. Updating the pending block in the db
    // while producing a block is not something we should test for. This is not a supported way of executing the block production service.
    // #[rstest::rstest]
    // #[tokio::test]
    // #[allow(clippy::too_many_arguments)]
    // async fn test_block_prod_on_pending_block_tick_updates_correct_block(
    //     #[future] devnet_setup: (
    //         Arc<MadaraBackend>,
    //         Arc<BlockProductionMetrics>,
    //         Arc<MockL1DataProvider>,
    //         Arc<Mempool>,
    //         Arc<TransactionValidator>,
    //         DevnetKeys,
    //     ),

    //     // Transactions
    //     #[from(tx_invoke_v0)]
    //     #[with(Felt::ZERO)]
    //     tx_invoke_v0: TxFixtureInfo,
    //     #[from(tx_l1_handler)]
    //     #[with(Felt::ONE)]
    //     tx_l1_handler: TxFixtureInfo,
    //     #[from(tx_declare_v0)]
    //     #[with(Felt::TWO)]
    //     tx_declare_v0: TxFixtureInfo,
    //     tx_deploy: TxFixtureInfo,
    //     tx_deploy_account: TxFixtureInfo,

    //     // Converted classes
    //     #[from(converted_class_legacy)]
    //     #[with(Felt::ZERO)]
    //     converted_class_legacy_0: mp_class::ConvertedClass,
    //     #[from(converted_class_sierra)]
    //     #[with(Felt::ONE, Felt::ONE)]
    //     converted_class_sierra_1: mp_class::ConvertedClass,
    //     #[from(converted_class_sierra)]
    //     #[with(Felt::TWO, Felt::TWO)]
    //     converted_class_sierra_2: mp_class::ConvertedClass,
    // ) {
    //     let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

    //     // ================================================================== //
    //     //               PART 1: add a transaction to the mempool             //
    //     // ================================================================== //

    //     // The transaction itself is meaningless, it's just to check
    //     // if the task correctly reads it and process it
    //     assert!(mempool.is_empty());
    //     sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //                PART 2: create block production task                //
    //     // ================================================================== //

    //     // Since there are no new pending blocks, this shouldn't
    //     // seal any blocks
    //     let block_production_task =
    //         BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
    //     block_production_task.close_pending_block_if_exists().await.unwrap();

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert_eq!(pending_block.inner.transactions.len(), 0);
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

    //     // ================================================================== //
    //     //                PART 3: call on pending time tick once              //
    //     // ================================================================== //

    //     let mut notifications = block_production_task.subscribe_state_notifications();
    //     AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });
    //     assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert!(mempool.is_empty());
    //     assert_eq!(pending_block.inner.transactions.len(), 1);
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

    //     // ================================================================== //
    //     //                  PART 4: we prepare the pending block              //
    //     // ================================================================== //

    //     let pending_inner = mp_block::MadaraBlockInner {
    //         transactions: vec![tx_invoke_v0.0, tx_l1_handler.0, tx_declare_v0.0, tx_deploy.0, tx_deploy_account.0],
    //         receipts: vec![tx_invoke_v0.1, tx_l1_handler.1, tx_declare_v0.1, tx_deploy.1, tx_deploy_account.1],
    //     };

    //     let pending_state_diff = mp_state_update::StateDiff {
    //         storage_diffs: vec![
    //             ContractStorageDiffItem {
    //                 address: Felt::ONE,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //             ContractStorageDiffItem {
    //                 address: Felt::TWO,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //             ContractStorageDiffItem {
    //                 address: Felt::THREE,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //         ],
    //         deprecated_declared_classes: vec![Felt::ZERO],
    //         declared_classes: vec![
    //             DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE },
    //             DeclaredClassItem { class_hash: Felt::TWO, compiled_class_hash: Felt::TWO },
    //         ],
    //         deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
    //         replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
    //         nonces: vec![
    //             NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
    //             NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
    //             NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
    //         ],
    //     };

    //     let converted_classes =
    //         vec![converted_class_legacy_0.clone(), converted_class_sierra_1.clone(), converted_class_sierra_2.clone()];

    //     // ================================================================== //
    //     //                 PART 5: storing the pending block                  //
    //     // ================================================================== //

    //     // We insert a pending block to check if the block production task
    //     // keeps a consistent state
    //     backend
    //         .store_block(
    //             mp_block::MadaraMaybePendingBlock {
    //                 info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
    //                     header: mp_block::header::PendingHeader::default(),
    //                     tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
    //                 }),
    //                 inner: pending_inner.clone(),
    //             },
    //             pending_state_diff.clone(),
    //             converted_classes.clone(),
    //         )
    //         .expect("Failed to store pending block");

    //     // ================================================================== //
    //     //           PART 6: add more transactions to the mempool             //
    //     // ================================================================== //

    //     sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //             PART 7: call on pending time tick again                //
    //     // ================================================================== //

    //     let mut notifications = block_production_task.subscribe_state_notifications();
    //     AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });
    //     assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);

    //     let pending_block = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert!(mempool.is_empty());
    //     assert_eq!(pending_block.inner.transactions.len(), 2);
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);
    // }

    // This test makes sure that the pending tick closes the block
    // if the bouncer capacity is reached
    #[rstest::rstest]
    #[tokio::test]
    #[traced_test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_closes_block(
        #[future]
        #[with(16, Duration::from_secs(60000), Duration::from_millis(1000), true)]
        devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add transactions to the mempool              //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ZERO).await;
        sign_and_add_invoke_tx(&contracts.0[1], &contracts.0[2], &backend, &tx_validator, Felt::ZERO).await;
        sign_and_add_invoke_tx(&contracts.0[2], &contracts.0[3], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: call on pending time tick                 //
        // ================================================================== //

        // The BouncerConfig is set up with amounts (100000) that should limit
        // the block size in a way that the pending tick on this task
        // closes the block
        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 2);

        let closed_block: mp_block::MadaraMaybePendingBlock =
            backend.get_block(&DbBlockId::Number(1)).unwrap().unwrap();
        assert_eq!(closed_block.inner.transactions.len(), 1);
        let closed_block: mp_block::MadaraMaybePendingBlock =
            backend.get_block(&DbBlockId::Number(2)).unwrap().unwrap();
        assert_eq!(closed_block.inner.transactions.len(), 1);
        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();
        assert_eq!(pending_block.inner.transactions.len(), 1);
        assert!(mempool.is_empty());
    }

    // This test makes sure that the block time tick correctly
    // adds the transaction to the pending block, closes it
    // and creates a new empty pending block
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_block_time_tick_closes_block(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // Since there are no new pending blocks, this shouldn't
        // seal any blocks
        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                      PART 3: call on block time                    //
        // ================================================================== //

        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );
        let notif = loop {
            let notif = notifications.recv().await.unwrap();
            if notif == BlockProductionStateNotification::UpdatedPendingBlock {
                continue;
            }
            break notif;
        };
        assert_eq!(notif, BlockProductionStateNotification::ClosedBlock);

        let pending_block = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    // // This test checks that the task fails to close the block
    // // if the block it's working on if forcibly change to one
    // // that isn't consistent with the previous state
    // // TODO(block_production_tests): this test does NOT make sense. Why are we testing that? This is not something we should allow. Forcibly changing
    // // the pending block state is not something we want to support, we should not test for that.
    // #[rstest::rstest]
    // #[tokio::test]
    // #[allow(clippy::too_many_arguments)]
    // async fn test_block_prod_on_block_time_fails_inconsistent_state(
    //     #[future] devnet_setup: (
    //         Arc<MadaraBackend>,
    //         Arc<BlockProductionMetrics>,
    //         Arc<MockL1DataProvider>,
    //         Arc<Mempool>,
    //         Arc<TransactionValidator>,
    //         DevnetKeys,
    //     ),

    //     // Transactions
    //     #[from(tx_invoke_v0)]
    //     #[with(Felt::ZERO)]
    //     tx_invoke_v0: TxFixtureInfo,
    //     #[from(tx_l1_handler)]
    //     #[with(Felt::ONE)]
    //     tx_l1_handler: TxFixtureInfo,
    //     #[from(tx_declare_v0)]
    //     #[with(Felt::TWO)]
    //     tx_declare_v0: TxFixtureInfo,
    //     tx_deploy: TxFixtureInfo,
    //     tx_deploy_account: TxFixtureInfo,

    //     // Converted classes
    //     #[from(converted_class_legacy)]
    //     #[with(Felt::ZERO)]
    //     converted_class_legacy_0: mp_class::ConvertedClass,
    //     #[from(converted_class_sierra)]
    //     #[with(Felt::ONE, Felt::ONE)]
    //     converted_class_sierra_1: mp_class::ConvertedClass,
    //     #[from(converted_class_sierra)]
    //     #[with(Felt::TWO, Felt::TWO)]
    //     converted_class_sierra_2: mp_class::ConvertedClass,
    // ) {
    //     let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

    //     // ================================================================== //
    //     //               PART 1: add a transaction to the mempool             //
    //     // ================================================================== //

    //     // The transaction itself is meaningless, it's just to check
    //     // if the task correctly reads it and process it
    //     assert!(mempool.is_empty());
    //     sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //                PART 2: create block production task                //
    //     // ================================================================== //

    //     // Since there are no new pending blocks, this shouldn't
    //     // seal any blocks
    //     let block_production_task =
    //         BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
    //     block_production_task.close_pending_block_if_exists().await.unwrap();

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert_eq!(pending_block.inner.transactions.len(), 0);
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

    //     // ================================================================== //
    //     //                  PART 3: we prepare the pending block              //
    //     // ================================================================== //

    //     let pending_inner = mp_block::MadaraBlockInner {
    //         transactions: vec![tx_invoke_v0.0, tx_l1_handler.0, tx_declare_v0.0, tx_deploy.0, tx_deploy_account.0],
    //         receipts: vec![tx_invoke_v0.1, tx_l1_handler.1, tx_declare_v0.1, tx_deploy.1, tx_deploy_account.1],
    //     };

    //     let pending_state_diff = mp_state_update::StateDiff {
    //         storage_diffs: vec![
    //             ContractStorageDiffItem {
    //                 address: Felt::ONE,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //             ContractStorageDiffItem {
    //                 address: Felt::TWO,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //             ContractStorageDiffItem {
    //                 address: Felt::THREE,
    //                 storage_entries: vec![
    //                     StorageEntry { key: Felt::ZERO, value: Felt::ZERO },
    //                     StorageEntry { key: Felt::ONE, value: Felt::ONE },
    //                     StorageEntry { key: Felt::TWO, value: Felt::TWO },
    //                 ],
    //             },
    //         ],
    //         deprecated_declared_classes: vec![Felt::ZERO],
    //         declared_classes: vec![
    //             DeclaredClassItem { class_hash: Felt::ONE, compiled_class_hash: Felt::ONE },
    //             DeclaredClassItem { class_hash: Felt::TWO, compiled_class_hash: Felt::TWO },
    //         ],
    //         deployed_contracts: vec![DeployedContractItem { address: Felt::THREE, class_hash: Felt::THREE }],
    //         replaced_classes: vec![ReplacedClassItem { contract_address: Felt::TWO, class_hash: Felt::TWO }],
    //         nonces: vec![
    //             NonceUpdate { contract_address: Felt::ONE, nonce: Felt::ONE },
    //             NonceUpdate { contract_address: Felt::TWO, nonce: Felt::TWO },
    //             NonceUpdate { contract_address: Felt::THREE, nonce: Felt::THREE },
    //         ],
    //     };

    //     let converted_classes =
    //         vec![converted_class_legacy_0.clone(), converted_class_sierra_1.clone(), converted_class_sierra_2.clone()];

    //     // ================================================================== //
    //     //                 PART 4: storing the pending block                  //
    //     // ================================================================== //

    //     // We insert a pending block to check if the block production task
    //     // keeps a consistent state
    //     backend
    //         .store_block(
    //             mp_block::MadaraMaybePendingBlock {
    //                 info: mp_block::MadaraMaybePendingBlockInfo::Pending(mp_block::MadaraPendingBlockInfo {
    //                     header: mp_block::header::PendingHeader::default(),
    //                     tx_hashes: vec![Felt::ONE, Felt::TWO, Felt::THREE],
    //                 }),
    //                 inner: pending_inner.clone(),
    //             },
    //             pending_state_diff.clone(),
    //             converted_classes.clone(),
    //         )
    //         .expect("Failed to store pending block");

    //     // ================================================================== //
    //     //                 PART 5: changing the task pending block            //
    //     // ================================================================== //

    //     // Here we purposefully change the block being worked on
    //     let (pending_block, classes) = get_pending_block_from_db(&backend).unwrap();
    //     // block_production_task.pending_block = Some(PendingBlockState {
    //     //     header: pending_block.header,
    //     //     transactions: pending_block.transactions,
    //     //     events: pending_block.events,
    //     //     declared_classes: classes,
    //     //     state_diff: pending_state_diff,
    //     // });

    //     // ================================================================== //
    //     //                      PART 6: call on block time                    //
    //     // ================================================================== //

    //     // If the program ran correctly, the pending block should
    //     // have no transactions on it after the method ran for at
    //     // least block_time

    //     wait_block_produced(block_production_task).await.unwrap();

    //     assert!(mempool.is_empty());
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    // }

    // This test checks when the block production task starts on
    // normal behaviour, it updates properly
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_normal_setup(
        #[future] devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // If the program ran correctly, the pending block should
        // have no transactions on it after the method ran for at
        // least block_time

        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let task_handle = tokio::spawn(async move {
            block_production_task.run(mp_utils::service::ServiceContext::new_for_testing()).await
        });

        // We abort after the minimum execution time
        // plus a little bit to guarantee
        tokio::time::sleep(std::time::Duration::from_secs(31)).await; // TODO(block_production_tests): do not rely on timeouts. timeouts are bad.
        task_handle.abort();

        let pending_block = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();
        let block_inner = backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;

        assert!(mempool.is_empty());
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(block_inner.transactions.len(), 1);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    // This test just verifies that pre validation checks and
    // balances are working as intended
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_pending_tick_too_small(
        #[future]
        #[with(16, Duration::from_secs(30), Duration::from_micros(1), false)]
        devnet_setup: (
            Arc<MadaraBackend>,
            Arc<BlockProductionMetrics>,
            Arc<MockL1DataProvider>,
            Arc<Mempool>,
            Arc<TransactionValidator>,
            DevnetKeys,
        ),
    ) {
        let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

        // ================================================================== //
        //             PART 1: we add a transaction to the mempool            //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // If the program ran correctly, the pending block should
        // have no transactions on it after the method ran for at
        // least block_time

        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
        block_production_task.close_pending_block_if_exists().await.unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let result = block_production_task
            .run(mp_utils::service::ServiceContext::new_for_testing())
            .await
            .expect_err("Should give an error");

        assert_eq!(result.to_string(), "Block time cannot be zero for block production.");
        assert!(!mempool.is_empty());
    }

    // // This test tries to overload the tokio tick system between
    // // updating pending block and closing it, but it should work properly
    // // TODO(block_production_tests): what are we testing here? I do not understand
    // #[rstest::rstest]
    // #[tokio::test]
    // #[allow(clippy::too_many_arguments)]
    // async fn test_block_prod_start_block_production_task_closes_block_right_after_pending(
    //     #[future]
    //     #[with(1, Duration::from_micros(1002), Duration::from_micros(1001), false)]
    //     devnet_setup: (
    //         Arc<MadaraBackend>,
    //         Arc<BlockProductionMetrics>,
    //         Arc<MockL1DataProvider>,
    //         Arc<Mempool>,
    //         Arc<TransactionValidator>,
    //         DevnetKeys,
    //     ),
    // ) {
    //     let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

    //     // ================================================================== //
    //     //             PART 1: we add a transaction to the mempool            //
    //     // ================================================================== //

    //     // The transaction itself is meaningless, it's just to check
    //     // if the task correctly reads it and process it
    //     assert!(mempool.is_empty());
    //     sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
    //     sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //                PART 2: create block production task                //
    //     // ================================================================== //

    //     let mut block_production_task =
    //         BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
    //     block_production_task.close_pending_block_if_exists().await.unwrap();

    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

    //     // ================================================================== //
    //     //                  PART 3: init block production task                //
    //     // ================================================================== //

    //     let mut notifications = block_production_task.subscribe_state_notifications();
    //     let _task = AbortOnDrop::spawn(async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() });
    //     assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::UpdatedPendingBlock);

    //     drop(task);

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     let block_inner = backend
    //         .get_block(&mp_block::BlockId::Number(1))
    //         .expect("Failed to retrieve latest block from db")
    //         .expect("Missing latest block")
    //         .inner;

    //     assert_eq!(block_inner.transactions.len(), 2);
    //     assert!(mempool.is_empty());
    //     assert!(pending_block.inner.transactions.is_empty());
    // }

    // // This test shuts down the block production task mid execution
    // // and creates a new one to check if it correctly set up the new state
    // // TODO(block_production_test): This is debatable whether this should work. A better test would be to test the fault-tolerance of the whole setup, which would mean
    // // we need to kill the MadaraBackend too.
    // #[rstest::rstest]
    // #[tokio::test]
    // #[allow(clippy::too_many_arguments)]
    // async fn test_block_prod_start_block_production_task_ungracious_shutdown_and_restart(
    //     #[future] devnet_setup: (
    //         Arc<MadaraBackend>,
    //         Arc<BlockProductionMetrics>,
    //         Arc<MockL1DataProvider>,
    //         Arc<Mempool>,
    //         Arc<TransactionValidator>,
    //         DevnetKeys,
    //     ),
    // ) {
    //     let (backend, metrics, l1_data_provider, mempool, tx_validator, contracts) = devnet_setup.await;

    //     // ================================================================== //
    //     //             PART 1: we add a transaction to the mempool            //
    //     // ================================================================== //

    //     // The transaction itself is meaningless, it's just to check
    //     // if the task correctly reads it and process it
    //     assert!(mempool.is_empty());
    //     sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
    //     sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //                PART 2: create block production task                //
    //     // ================================================================== //

    //     let block_production_task = BlockProductionTask::new(
    //         Arc::clone(&backend),
    //         Arc::clone(&mempool),
    //         metrics.clone(),
    //         l1_data_provider.clone(),
    //     )
    //     .unwrap();
    //     block_production_task.close_pending_block_if_exists().await.unwrap();

    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

    //     // ================================================================== //
    //     //                PART 3: init block production task                  //
    //     // ================================================================== //

    //     let ctx = mp_utils::service::ServiceContext::new_for_testing();

    //     let task_handle = tokio::spawn(async move { block_production_task.run(ctx).await });
    //     tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    //     task_handle.abort();

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert!(mempool.is_empty());
    //     assert_eq!(pending_block.inner.transactions.len(), 2);

    //     // ================================================================== //
    //     //           PART 4: we add more transactions to the mempool          //
    //     // ================================================================== //

    //     // The transaction itself is meaningless, it's just to check
    //     // if the task correctly reads it and process it
    //     sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::TWO).await;
    //     assert!(!mempool.is_empty());

    //     // ================================================================== //
    //     //                PART 5: create block production task                //
    //     // ================================================================== //

    //     // This should seal the previous pending block
    //     let block_production_task =
    //         BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider);
    //     block_production_task.close_pending_block_if_exists().await.unwrap();

    //     let block_inner = backend
    //         .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
    //         .expect("Failed to retrieve latest block from db")
    //         .expect("Missing latest block")
    //         .inner;

    //     assert_eq!(block_inner.transactions.len(), 2);
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);

    //     // ================================================================== //
    //     //             PART 6: we start block production again                //
    //     // ================================================================== //

    //     let task_handle = tokio::spawn(async move {
    //         block_production_task.run(mp_utils::service::ServiceContext::new_for_testing()).await
    //     });

    //     tokio::time::sleep(std::time::Duration::from_secs(35)).await;
    //     task_handle.abort();

    //     let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

    //     assert!(mempool.is_empty());
    //     assert!(pending_block.inner.transactions.is_empty());
    //     assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 2);
    // }
}
