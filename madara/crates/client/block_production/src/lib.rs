//! Block production service.

use crate::batcher::Batcher;
use crate::metrics::BlockProductionMetrics;
use anyhow::Context;
use blockifier::state::cached_state::{CommitmentStateDiff, StateMaps, StorageEntry, StorageView};
use executor::{BatchExecutionResult, ExecutorMessage};
use futures::future::OptionFuture;
use mc_db::db_block_id::DbBlockId;
use mc_db::MadaraBackend;
use mc_exec::execution::TxInfo;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_block::header::PendingHeader;
use mp_block::{BlockId, BlockTag, PendingFullBlock, TransactionWithReceipt};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::{from_blockifier_execution_info, EventWithTransactionHash};
use mp_state_update::{DeclaredClassItem, NonceUpdate};
use mp_transactions::validated::ValidatedMempoolTx;
use mp_transactions::TransactionWithHash;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use opentelemetry::KeyValue;
use starknet_types_core::felt::Felt;
use std::collections::{HashMap, HashSet};
use std::mem;
use std::sync::Arc;
use std::time::Instant;
use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
use starknet_api::state::StorageKey;
use tokio::sync::mpsc;
use util::{BlockExecutionContext, ExecutionStats};

mod batcher;
mod executor;
mod handle;
pub mod metrics;
mod util;

pub use handle::BlockProductionHandle;

#[derive(Debug, Clone)]
pub struct PendingBlockState {
    pub header: PendingHeader,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
    pub declared_classes: Vec<ConvertedClass>,
    /// Unnormalized state diffs.
    pub state: StateMaps,
    pub consumed_core_contract_nonces: HashSet<u64>,
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
            state: StateMaps { storage: initial_state_diffs_storage, ..Default::default() },
            transactions: vec![],
            events: vec![],
            declared_classes: vec![],
            consumed_core_contract_nonces: Default::default(),
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
                state_diff: mc_exec::state_diff::create_normalized_state_diff(backend, &on_top_of_block_id, self.state, self.declared_classes.clone())
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
    backend: Arc<MadaraBackend>,
    pub block: PendingBlockState,
    pub block_n: u64,
    // These are reset every pending tick.
    pub tx_executed_for_tick: Vec<Felt>,
    pub stats_for_tick: ExecutionStats,
}

impl CurrentPendingState {
    pub fn new(backend: Arc<MadaraBackend>, block: PendingBlockState, block_n: u64) -> Self {
        Self { backend, block, block_n, tx_executed_for_tick: Default::default(), stats_for_tick: Default::default() }
    }
    /// Process the execution result, merging it with the current pending state
    pub fn append_batch(&mut self, batch: BatchExecutionResult) {
        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            self.tx_executed_for_tick.push(blockifier_tx.tx_hash().to_felt());

            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.block
                    .consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().expect("Invalid nonce"));
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                if let Some(class) = additional_info.declared_class.take() {
                    if !execution_info.is_reverted() {
                        self.block.declared_classes.push(class);
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
                self.block.state.extend(&state_diff);
                let tx = TransactionWithReceipt { transaction: converted_tx.transaction, receipt };
                self.block.transactions.push(tx.clone());
                self.backend.on_new_pending_tx(tx)
            }
        }
        self.stats_for_tick += batch.stats;
    }
}

/// Used for listening to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock,
    UpdatedPendingBlock,
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
pub(crate) enum TaskState {
    NotExecuting {
        /// [`None`] when the next block to execute is genesis.
        latest_block_n: Option<u64>,
        /// [`Felt::ZERO`] when the next block to execute is genesis.
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
    mempool: Arc<Mempool>,
    current_state: Option<TaskState>,
    metrics: Arc<BlockProductionMetrics>,
    state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
    handle: BlockProductionHandle,
    executor_commands_recv: Option<mpsc::UnboundedReceiver<executor::ExecutorCommand>>,
    l1_client: Arc<dyn SettlementClient>,
    bypass_tx_input: Option<mpsc::Receiver<ValidatedMempoolTx>>,
}

impl BlockProductionTask {
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
    ) -> Self {
        let (sender, recv) = mpsc::unbounded_channel();
        let (bypass_input_sender, bypass_tx_input) = mpsc::channel(16);
        Self {
            backend: backend.clone(),
            mempool,
            current_state: None,
            metrics,
            handle: BlockProductionHandle::new(backend, sender, bypass_input_sender),
            state_notifications: None,
            executor_commands_recv: Some(recv),
            l1_client,
            bypass_tx_input: Some(bypass_tx_input),
        }
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

        // let (block, declared_classes) = get_pending_block_from_db(&self.backend)?;

        self.backend.clear_pending_block().context("Error clearing pending block")?;

        // let block_n = self.backend.get_latest_block_n().context("Getting latest block n")?.map(|n| n + 1).unwrap_or(0);
        // self.close_and_save_block(block_n, block, declared_classes, vec![]).await?;

        Ok(())
    }

    /// Returns the block_hash.
    #[tracing::instrument(skip(self, block, classes))]
    async fn close_and_save_block(
        &mut self,
        block_n: u64,
        block: PendingFullBlock,
        classes: Vec<ConvertedClass>,
        _txs_executed: Vec<Felt>,
    ) -> anyhow::Result<Felt> {
        tracing::debug!("Close and save block block_n={block_n}");
        let start_time = Instant::now();

        let n_txs = block.transactions.len();


        // println!("This is inside close_and_save_block that calls add_full_block_with_classes ");
        // Close and import the block
        let block_hash = self
            .backend
            .add_full_block_with_classes(block, block_n, &classes, /* pre_v0_13_2_hash_override */ true)
            .await
            .context("Error closing block")?;

        let time_to_close = start_time.elapsed();
        tracing::info!("⛏️  Closed block #{block_n} with {n_txs} transactions - {time_to_close:?}");

        // Record metrics
        let attributes = [
            KeyValue::new("transactions_added", n_txs.to_string()),
            KeyValue::new("closing_time", time_to_close.as_secs_f32().to_string()),
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
                tracing::debug!("Received ExecutorMessage::StartNewBlock block_n={}", exec_ctx.block_n);
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::NotExecuting { latest_block_n, latest_block_hash } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let new_block_n = latest_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                if new_block_n != exec_ctx.block_n {
                    anyhow::bail!(
                        "Received new block_n={} from executor, expected block_n={}",
                        exec_ctx.block_n,
                        new_block_n
                    )
                }

                self.current_state = Some(TaskState::Executing(
                    CurrentPendingState::new(
                        Arc::clone(&self.backend),
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
                    "Received ExecutorMessage::BatchExecuted executed_txs={:?}",
                    batch_execution_result.executed_txs
                );
                let current_state = self.current_state.as_mut().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };
                state.append_batch(batch_execution_result);
            }
            ExecutorMessage::EndBlock(block_exec_summary) => {
                tracing::debug!("Received ExecutorMessage::EndBlock");
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::Executing(mut state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                // pub struct CommitmentStateDiff {
                //     // Contract instance attributes (per address).
                //     pub address_to_class_hash: IndexMap<ContractAddress, ClassHash>,
                //     pub address_to_nonce: IndexMap<ContractAddress, Nonce>,
                //     pub storage_updates: IndexMap<ContractAddress, IndexMap<StorageKey, Felt>>,
                //
                //     // Global attributes.
                //     pub class_hash_to_compiled_class_hash: IndexMap<ClassHash, CompiledClassHash>,
                // }

                // pub struct StateMaps {
                //     pub nonces: HashMap<ContractAddress, Nonce>,
                //     pub class_hashes: HashMap<ContractAddress, ClassHash>,
                //     pub storage: HashMap<StorageEntry, Felt>,
                //     pub compiled_class_hashes: HashMap<ClassHash, CompiledClassHash>,
                //     pub declared_contracts: HashMap<ClassHash, bool>,
                // }

                // I need to convert from CommitmentStateDiff to StateMaps
                // I have StateMaps to CommitmentStateDiff available

                // impl From<StateMaps> for CommitmentStateDiff {
                //     fn from(diff: StateMaps) -> Self {
                //         Self {
                //             address_to_class_hash: IndexMap::from_iter(diff.class_hashes),
                //             storage_updates: StorageDiff::from(StorageView(diff.storage)),
                //             class_hash_to_compiled_class_hash: IndexMap::from_iter(diff.compiled_class_hashes),
                //             address_to_nonce: IndexMap::from_iter(diff.nonces),
                //         }
                //     }
                // }

                let new_state: StateMaps = block_exec_summary.state_diff.into();

                let new_block: PendingBlockState = PendingBlockState {
                    header : state.block.header.clone(),
                    transactions: state.block.transactions.clone(),
                    events: state.block.events.clone(),
                    declared_classes: state.block.declared_classes.clone(),
                    consumed_core_contract_nonces: state.block.consumed_core_contract_nonces.clone(),
                    state: new_state,
                };

                state.block = new_block;

                self.mempool
                    .on_tx_batch_executed(
                        state.block.state.nonces.iter().map(|(contract_address, nonce)| NonceUpdate {
                            contract_address: contract_address.to_felt(),
                            nonce: nonce.to_felt(),
                        }),
                        state.tx_executed_for_tick.iter().cloned(),
                    )
                    .await
                    .context("Updating mempool state")?;

                let (block, classes) = state.block.into_full_block_with_classes(&self.backend, state.block_n)?;
                // println!("This is where we process reply");
                let block_hash = self
                    .close_and_save_block(state.block_n, block, classes, state.tx_executed_for_tick)
                    .await
                    .context("Closing and saving block")?;

                self.current_state = Some(TaskState::NotExecuting {
                    latest_block_n: Some(state.block_n),
                    latest_block_hash: block_hash,
                });
            }
        }

        Ok(())
    }

    async fn store_pending_block(&mut self) -> anyhow::Result<()> {
        if let TaskState::Executing(state) = self.current_state.as_mut().context("No current state")? {
            for l1_nonce in &state.block.consumed_core_contract_nonces {
                // This ensures we remove the nonces for rejected L1 to L2 message transactions. This avoids us from reprocessing them on restart.
                self.backend
                    .remove_pending_message_to_l2(*l1_nonce)
                    .context("Removing pending message to l2 from database")?;
            }

            let (block, classes) = state
                .block
                .clone()
                .into_full_block_with_classes(&self.backend, state.block_n)
                .context("Converting to full pending block")?;
            self.backend.store_pending_block_with_classes(block, &classes)?;

            self.mempool
                .on_tx_batch_executed(
                    state.block.state.nonces.iter().map(|(contract_address, nonce)| NonceUpdate {
                        contract_address: contract_address.to_felt(),
                        nonce: nonce.to_felt(),
                    }),
                    state.tx_executed_for_tick.iter().cloned(),
                )
                .await
                .context("Updating mempool state")?;

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

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_pending_block_if_exists().await.context("Cannot close pending block on startup")?;

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
        self.current_state = Some(TaskState::NotExecuting { latest_block_n, latest_block_hash });

        Ok(())
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.setup_initial_state().await?;

        let mut executor = executor::start_executor_thread(
            Arc::clone(&self.backend),
            self.executor_commands_recv.take().context("Task already started")?,
        )
        .context("Starting executor thread")?;

        let mut interval_pending_block_update = self.backend.chain_config().pending_block_update_time.map(|t| {
            let mut int = tokio::time::interval(t);
            int.reset(); // Skip the immediate first tick.
            int.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            int
        });

        // Batcher task is handled in a separate tokio task.
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        let mut batcher_task = AbortOnDrop::spawn(
            Batcher::new(
                self.backend.clone(),
                self.mempool.clone(),
                self.l1_client.clone(),
                ctx,
                batch_sender,
                bypass_tx_input,
            )
            .run(),
        );

        // Graceful shutdown: when the service is asked to stop, the `batcher_task` will stop,
        //  which will close the `send_batch` channel (by dropping it). The executor thread then will see that the channel
        //  is closed next time it tries to receive from it. The executor thread shuts down, dropping the `executor.stop` channel,
        //  therefore closing it as well.
        // We will then see the anyhow::Ok(()) result in the stop channel, as per the implementation of [`StopErrorReceiver::recv`].
        // Note that for this to work, we need to make sure the `send_batch` channel is never aliased -
        //  otherwise it will never not be closed automatically.

        loop {
            tokio::select! {

                // Bubble up errors from the batcher task. (tokio JoinHandle)
                res = &mut batcher_task => return res.context("In batcher task"),

                // Process results from the execution
                Some(reply) = executor.replies.recv() => {
                    self.process_reply(reply).await.context("Processing reply from executor thread")?;
                }

                // Update the pending block in db periodically.
                Some(_) = OptionFuture::from(interval_pending_block_update.as_mut().map(|int| int.tick())) => {
                    self.store_pending_block().await.context("Storing pending block")?;
                }

                // Bubble up errors from the executor thread, or graceful shutdown.
                // We do this after processing all the replies to ensure we don't lose some of the state by accident.
                res = executor.stop.recv() => return res.context("In executor thread"),
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::BlockProductionStateNotification;
    use crate::{metrics::BlockProductionMetrics, BlockProductionTask};
    use blockifier::{
        bouncer::{BouncerConfig, BouncerWeights},
        state::cached_state::StateMaps,
    };
    use mc_db::{db_block_id::DbBlockId, MadaraBackend};
    use mc_devnet::{
        Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
    };
    use mc_mempool::{Mempool, MempoolConfig};
    use mc_settlement_client::L1ClientMock;
    use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
    use mp_block::{BlockId, BlockTag};
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use mp_receipt::{Event, ExecutionResult};
    use mp_rpc::{
        BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, DaMode,
        InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
    };
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
        StorageEntry,
    };
    use mp_transactions::compute_hash::calculate_contract_address;
    use mp_transactions::IntoStarknetApiExt;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
    use mp_utils::service::ServiceContext;
    use mp_utils::AbortOnDrop;
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
    use starknet_core::utils::get_selector_from_name;
    use starknet_types_core::felt::Felt;
    use std::{collections::HashMap, sync::Arc, time::Duration};

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
            l1_gas: 1000000,
            message_segment_length: 10000,
            n_events: 10000,
            state_diff_size: 10000,
            ..BouncerWeights::max()
        }
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
            )
        }
    }

    #[rstest::fixture]
    pub async fn devnet_setup(
        #[default(Duration::from_secs(30))] block_time: Duration,
        #[default(Some(Duration::from_secs(2)))] pending_block_update_time: Option<Duration>,
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
                pending_block_update_time,
                bouncer_config: BouncerConfig { block_max_capacity: bouncer_weights },
                ..ChainConfig::madara_devnet()
            })
        } else {
            Arc::new(ChainConfig { block_time, pending_block_update_time, ..ChainConfig::madara_devnet() })
        };

        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        backend.set_l1_gas_quote_for_testing();
        genesis.build_and_store(&backend).await.unwrap();

        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
        let tx_validator = Arc::new(TransactionValidator::new(
            Arc::clone(&mempool) as _,
            Arc::clone(&backend),
            TransactionValidatorConfig::default(),
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
                compiled_class_hash,
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

        let (compiled_contract_class_hash, _compiled_class) = flattened_class.compile_to_casm().unwrap();

        let mut declare_txn: BroadcastedDeclareTxn = BroadcastedDeclareTxn::V3(BroadcastedDeclareTxnV3 {
            sender_address: contract.address,
            compiled_class_hash: compiled_contract_class_hash,
            // this field will be filled below
            signature: vec![].into(),
            nonce,
            contract_class: flattened_class.into(),
            resource_bounds: ResourceBoundsMapping {
                l1_gas: ResourceBounds { max_amount: 220000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
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
                l2_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
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

    #[rstest::rstest]
    fn test_block_prod_state_map_to_state_diff(backend: Arc<MadaraBackend>) {
        let mut nonces = HashMap::new();
        nonces.insert(Felt::from_hex_unchecked("1").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("1")));
        nonces.insert(Felt::from_hex_unchecked("2").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("2")));
        nonces.insert(Felt::from_hex_unchecked("3").try_into().unwrap(), Nonce(Felt::from_hex_unchecked("3")));

        let mut class_hashes = HashMap::new();
        class_hashes
            .insert(Felt::from_hex_unchecked("1").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a551")));
        class_hashes
            .insert(Felt::from_hex_unchecked("2").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a552")));
        class_hashes
            .insert(Felt::from_hex_unchecked("3").try_into().unwrap(), ClassHash(Felt::from_hex_unchecked("0xc1a553")));

        let mut storage = HashMap::new();
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("1").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("2").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("1").try_into().unwrap()),
            Felt::from_hex_unchecked("1"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("2").try_into().unwrap()),
            Felt::from_hex_unchecked("2"),
        );
        storage.insert(
            (Felt::from_hex_unchecked("3").try_into().unwrap(), Felt::from_hex_unchecked("3").try_into().unwrap()),
            Felt::from_hex_unchecked("3"),
        );

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a551")),
            CompiledClassHash(Felt::from_hex_unchecked("0x1")),
        );
        compiled_class_hashes.insert(
            ClassHash(Felt::from_hex_unchecked("0xc1a552")),
            CompiledClassHash(Felt::from_hex_unchecked("0x2")),
        );

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a551")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a552")), true);
        declared_contracts.insert(ClassHash(Felt::from_hex_unchecked("0xc1a553")), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("1"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("2"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: Felt::from_hex_unchecked("3"),
                storage_entries: vec![
                    StorageEntry { key: Felt::from_hex_unchecked("1"), value: Felt::ONE },
                    StorageEntry { key: Felt::from_hex_unchecked("2"), value: Felt::TWO },
                    StorageEntry { key: Felt::from_hex_unchecked("3"), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![Felt::from_hex_unchecked("0xc1a553")];

        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
                compiled_class_hash: Felt::from_hex_unchecked("0x1"),
            },
            DeclaredClassItem {
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
                compiled_class_hash: Felt::from_hex_unchecked("0x2"),
            },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: Felt::from_hex_unchecked("1"), nonce: Felt::from_hex_unchecked("1") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("2"), nonce: Felt::from_hex_unchecked("2") },
            NonceUpdate { contract_address: Felt::from_hex_unchecked("3"), nonce: Felt::from_hex_unchecked("3") },
        ];

        let deployed_contracts = vec![
            DeployedContractItem {
                address: Felt::from_hex_unchecked("1"),
                class_hash: Felt::from_hex_unchecked("0xc1a551"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("2"),
                class_hash: Felt::from_hex_unchecked("0xc1a552"),
            },
            DeployedContractItem {
                address: Felt::from_hex_unchecked("3"),
                class_hash: Felt::from_hex_unchecked("0xc1a553"),
            },
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

        let mut actual =
            mc_exec::state_diff::create_normalized_state_diff(&backend, &Option::<_>::None, state_map).unwrap();

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
        #[future] devnet_setup: DevnetSetup,
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
        let mut devnet_setup = devnet_setup.await;

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

        devnet_setup
            .backend
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
        let mut block_production_task = devnet_setup.block_prod_task();
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap(), Some(0));
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check this was the case.
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap(), Some(1));

        let block_inner = devnet_setup
            .backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;
        assert_eq!(block_inner, pending_inner);

        let state_diff = devnet_setup
            .backend
            .get_block_state_diff(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest state diff from db")
            .expect("Missing latest state diff");
        assert_eq!(state_diff, pending_state_diff);

        let class = devnet_setup
            .backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ZERO)
            .expect("Failed to retrieve class at hash 0x0 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_legacy_0);

        let class = devnet_setup
            .backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ONE)
            .expect("Failed to retrieve class at hash 0x1 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_1);

        let class = devnet_setup
            .backend
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
        #[future] devnet_setup: DevnetSetup,

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
        let mut devnet_setup = devnet_setup.await;

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
        devnet_setup
            .backend
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
        devnet_setup
            .backend
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
        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check this was the case.
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 1);

        // Block 0 should not have been overridden!
        let block = devnet_setup
            .backend
            .get_block(&mp_block::BlockId::Number(0))
            .expect("Failed to retrieve block 0 from db")
            .expect("Missing block 0");

        assert_eq!(block.info.as_closed().unwrap().header.parent_block_hash, Felt::ZERO);
        assert_eq!(block.inner, ready_inner);

        let block = devnet_setup
            .backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block");

        assert_eq!(block.info.as_closed().unwrap().header.parent_block_hash, Felt::ZERO);
        assert_eq!(block.inner, pending_inner);

        // Block 0 should not have been overridden!
        let state_diff = devnet_setup
            .backend
            .get_block_state_diff(&mp_block::BlockId::Number(0))
            .expect("Failed to retrieve state diff at block 0 from db")
            .expect("Missing state diff at block 0");
        assert_eq!(ready_state_diff, state_diff);

        let state_diff = devnet_setup
            .backend
            .get_block_state_diff(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest state diff from db")
            .expect("Missing latest state diff");
        assert_eq!(pending_state_diff, state_diff);

        let class = devnet_setup
            .backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ZERO)
            .expect("Failed to retrieve class at hash 0x0 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_legacy_0);

        let class = devnet_setup
            .backend
            .get_converted_class(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest), &Felt::ONE)
            .expect("Failed to retrieve class at hash 0x1 from db")
            .expect("Missing class at index 0x0");
        assert_eq!(class, converted_class_sierra_1);

        let class = devnet_setup
            .backend
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
    #[tokio::test]
    async fn block_prod_pending_close_on_startup_no_pending(#[future] devnet_setup: DevnetSetup) {
        let mut devnet_setup = devnet_setup.await;

        // Simulates starting block production without a pending block in db
        let mut block_production_task = devnet_setup.block_prod_task();
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap(), Some(0)); // there is a genesis block in the db.
        block_production_task.close_pending_block_if_exists().await.unwrap();

        // Now we check no block was added to the db
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap(), Some(0));
    }

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing class.
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class(
        #[future] devnet_setup: DevnetSetup,
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let mut devnet_setup = devnet_setup.await;

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

        devnet_setup
            .backend
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
        let mut block_production_task = devnet_setup.block_prod_task();
        let err = block_production_task.close_pending_block_if_exists().await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing legacy class.
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class_legacy(
        #[future] devnet_setup: DevnetSetup,
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let mut devnet_setup = devnet_setup.await;

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

        devnet_setup
            .backend
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
        let mut block_production_task = devnet_setup.block_prod_task();
        let err = block_production_task.close_pending_block_if_exists().await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    // This test makes sure that the pending tick updates
    // the pending block correctly without closing if the bouncer limits aren't reached
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_block_still_pending(#[future] devnet_setup: DevnetSetup) {
        let mut devnet_setup = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(devnet_setup.mempool.is_empty().await);
        sign_and_add_declare_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        assert!(!devnet_setup.mempool.is_empty().await);

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // Since there are no new pending blocks, this shouldn't
        // seal any blocks
        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);

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

        let pending_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(devnet_setup.mempool.is_empty().await);
        assert_eq!(pending_block.inner.transactions.len(), 1);
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);
    }

    // This test makes sure that the pending tick closes the block
    // if the bouncer capacity is reached
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_closes_block(
        #[future]
        #[with(Duration::from_secs(1), None, true)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add transactions to the mempool              //
        // ================================================================== //

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

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);

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

        let closed_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Number(1)).unwrap().unwrap();
        assert_eq!(closed_block.inner.transactions.len(), 3);
        let closed_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Number(2)).unwrap().unwrap();
        assert_eq!(closed_block.inner.transactions.len(), 0);
        assert!(devnet_setup.mempool.is_empty().await);
    }

    // This test makes sure that the block time tick correctly
    // adds the transaction to the pending block, closes it
    // and creates a new empty pending block
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_block_time_tick_closes_block(#[future] devnet_setup: DevnetSetup) {
        let mut devnet_setup = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(devnet_setup.mempool.is_empty().await);
        sign_and_add_declare_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        assert!(!devnet_setup.mempool.is_empty().await);

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // Since there are no new pending blocks, this shouldn't
        // seal any blocks
        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock =
            devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);

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

        let pending_block = devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(devnet_setup.mempool.is_empty().await);
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    // This test checks when the block production task starts on
    // normal behaviour, it updates properly
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_normal_setup(#[future] devnet_setup: DevnetSetup) {
        let mut devnet_setup = devnet_setup.await;

        // ================================================================== //
        //               PART 1: add a transaction to the mempool             //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(devnet_setup.mempool.is_empty().await);
        sign_and_add_declare_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        assert!(!devnet_setup.mempool.is_empty().await);

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // If the program ran correctly, the pending block should
        // have no transactions on it after the method ran for at
        // least block_time

        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);

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

        let pending_block = devnet_setup.backend.get_block(&DbBlockId::Pending).unwrap().unwrap();
        let block_inner = devnet_setup
            .backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;

        assert!(devnet_setup.mempool.is_empty().await);
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(block_inner.transactions.len(), 1);
        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    // This test just verifies that pre validation checks and
    // balances are working as intended
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_pending_tick_too_small(
        #[future]
        #[with(Duration::from_secs(30), Some(Duration::default()), false)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        // ================================================================== //
        //             PART 1: we add a transaction to the mempool            //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(devnet_setup.mempool.is_empty().await);
        sign_and_add_declare_tx(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            &devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;
        assert!(!devnet_setup.mempool.is_empty().await);

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // If the program ran correctly, the pending block should
        // have no transactions on it after the method ran for at
        // least block_time

        let mut block_production_task = devnet_setup.block_prod_task();
        block_production_task.close_pending_block_if_exists().await.unwrap();

        assert_eq!(devnet_setup.backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let result = block_production_task
            .run(mp_utils::service::ServiceContext::new_for_testing())
            .await
            .expect_err("Should give an error");

        assert_eq!(result.to_string(), "Pending block update time cannot be zero for block production.");
        assert!(!devnet_setup.mempool.is_empty().await);
    }

    #[rstest::rstest]
    #[tokio::test]
    async fn test_state_diff_has_block_n_min_10(
        #[future]
        #[with(Duration::from_secs(3000000000), None, false)]
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

        // genesis already deployed
        for n in 1..10 {
            control.close_block().await.unwrap();
            tracing::debug!("closed block");
            assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

            // Empty state diff
            assert_eq!(
                devnet_setup.backend.get_block_state_diff(&DbBlockId::Number(n)).unwrap().unwrap(),
                StateDiff::default()
            );
        }

        // Hash for block_10
        control.close_block().await.unwrap();
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        let block_hash_0 = devnet_setup.backend.get_block_hash(&DbBlockId::Number(0)).unwrap().unwrap();
        assert_eq!(
            devnet_setup.backend.get_block_state_diff(&DbBlockId::Number(10)).unwrap().unwrap(),
            StateDiff {
                storage_diffs: vec![ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![StorageEntry { key: 0.into(), value: block_hash_0 }]
                }],
                ..Default::default()
            }
        );

        // Hash for block_11
        control.close_block().await.unwrap();
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        let block_hash_1 = devnet_setup.backend.get_block_hash(&DbBlockId::Number(1)).unwrap().unwrap();
        assert_eq!(
            devnet_setup.backend.get_block_state_diff(&DbBlockId::Number(11)).unwrap().unwrap(),
            StateDiff {
                storage_diffs: vec![ContractStorageDiffItem {
                    address: Felt::ONE,
                    storage_entries: vec![StorageEntry { key: 1.into(), value: block_hash_1 }]
                }],
                ..Default::default()
            }
        );
    }

    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    async fn test_l1_handler_tx(
        #[future]
        #[with(Duration::from_secs(3000000000), Some(Duration::from_millis(500)), false)]
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

        while devnet_setup.backend.latest_pending_block().tx_hashes.is_empty() {
            notifications.recv().await.unwrap();
        }
        assert_eq!(
            devnet_setup.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap().inner.receipts[0]
                .execution_result(),
            ExecutionResult::Succeeded
        );
        control.close_block().await.unwrap();
        while !devnet_setup.backend.latest_pending_block().tx_hashes.is_empty() {
            notifications.recv().await.unwrap();
        }

        // Deploy contract through UDC.

        let (contract_address, tx) = make_udc_call(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            /* nonce */ Felt::ONE,
            res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        devnet_setup.tx_validator.submit_invoke_transaction(tx).await.unwrap();

        while devnet_setup.backend.latest_pending_block().tx_hashes.is_empty() {
            notifications.recv().await.unwrap();
        }
        assert_eq!(
            devnet_setup.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap().inner.receipts[0]
                .execution_result(),
            ExecutionResult::Succeeded
        );
        control.close_block().await.unwrap();
        while !devnet_setup.backend.latest_pending_block().tx_hashes.is_empty() {
            notifications.recv().await.unwrap();
        }

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

        while devnet_setup.backend.latest_pending_block().tx_hashes.is_empty() {
            notifications.recv().await.unwrap();
        }

        let receipt = devnet_setup.backend.get_block(&BlockId::Tag(BlockTag::Pending)).unwrap().unwrap().inner.receipts
            [0]
        .clone();
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
}
