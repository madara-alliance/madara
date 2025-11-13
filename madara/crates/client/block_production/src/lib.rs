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
//! ## Pending Phase
//!
//! One important detail to note is that the [`PendingBlockState`] kept in the
//! [`BlockProductionTask`] is stored in **RAM**. We periodically flush this value to db at an
//! interval defined by the `pending tick` as set in the chain config.
//! (TODO(mohit 13/10/2025): update this when 0.14.0 merges)
//!
//! [mempool]: mc_mempool
//! [`StartNewBlock`]: ExecutorMessage::StartNewBlock
//! [`BatchExecuted`]: ExecutorMessage::BatchExecuted
//! [`EndBlock`]: ExecutorMessage::EndBlock
//! [`ExecutorThreadHandle::send_batch`]: executor::ExecutorThreadHandle::send_batch
//! [`ExecutorThread::incoming_batches`]: executor::thread::ExecutorThread::incoming_batches
//! [`ExecutorThread`]: executor::thread::ExecutorThread
//! [`process_reply`]: BlockProductionTask::process_reply

use crate::batcher::Batcher;
use crate::metrics::BlockProductionMetrics;
use crate::util::BlockExecutionContext;
use anyhow::Context;
use blockifier::blockifier::transaction_executor::BlockExecutionSummary;
use executor::{BatchExecutionResult, ExecutorMessage};
use mc_db::preconfirmed::{PreconfirmedBlock, PreconfirmedExecutedTransaction};
use mc_db::{MadaraBackend, MadaraPreconfirmedBlockView, MadaraStateView};
use mc_exec::execution::TxInfo;
use mc_exec::LayeredStateAdapter;
use mc_mempool::Mempool;
use mc_settlement_client::SettlementClient;
use mp_block::TransactionWithReceipt;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{ClassUpdateItem, DeclaredClassCompiledClass, TransactionStateUpdate};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::TransactionWithHash;
use mp_utils::rayon::global_spawn_rayon_task;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
use opentelemetry::KeyValue;
use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};
use tokio::sync::mpsc;

mod batcher;
mod executor;
mod handle;
pub mod metrics;
mod util;

pub use handle::BlockProductionHandle;

/// Used for listening to state changes in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockProductionStateNotification {
    ClosedBlock,
    BatchExecuted,
}

#[derive(Debug)]
pub(crate) struct CurrentBlockState {
    backend: Arc<MadaraBackend>,
    pub block_number: u64,
    pub consumed_core_contract_nonces: HashSet<u64>,
    /// We need to keep track of deployed contracts, because blockifier can't make the difference between replaced class / deployed contract :/
    pub deployed_contracts: HashSet<Felt>,
}

impl CurrentBlockState {
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self {
            backend,
            block_number,
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
        }
    }
    /// Process the execution result, merging it with the current pending state
    pub async fn append_batch(&mut self, mut batch: BatchExecutionResult) -> anyhow::Result<()> {
        let mut executed = vec![];

        for ((blockifier_exec_result, blockifier_tx), mut additional_info) in
            batch.blockifier_results.into_iter().zip(batch.executed_txs.txs).zip(batch.executed_txs.additional_info)
        {
            if let Some(core_contract_nonce) = blockifier_tx.l1_handler_tx_nonce() {
                // Even when the l1 handler tx is reverted, we mark the nonce as consumed.
                self.consumed_core_contract_nonces
                    .insert(core_contract_nonce.to_felt().try_into().expect("Invalid nonce"));
            }

            if let Ok((execution_info, state_diff)) = blockifier_exec_result {
                let declared_class = additional_info.declared_class.take().filter(|_| !execution_info.is_reverted());

                let receipt = from_blockifier_execution_info(&execution_info, &blockifier_tx);
                let converted_tx = TransactionWithHash::from(blockifier_tx.clone());

                // Extract paid_fee_on_l1 from L1 handler transactions
                let paid_fee_on_l1 = match &blockifier_tx {
                    blockifier::transaction::transaction_execution::Transaction::L1Handler(l1_tx) => {
                        Some(l1_tx.paid_fee_on_l1.0)
                    }
                    _ => None,
                };

                executed.push(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt { transaction: converted_tx.transaction, receipt },
                    state_diff: TransactionStateUpdate {
                        nonces: state_diff
                            .nonces
                            .into_iter()
                            .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
                            .collect(),
                        contract_class_hashes: state_diff
                            .class_hashes
                            .into_iter()
                            .map(|(contract_addr, class_hash)| {
                                let entry = if !self.deployed_contracts.contains(&contract_addr)
                                    && !self.backend.view_on_latest_confirmed().is_contract_deployed(&contract_addr)?
                                {
                                    self.deployed_contracts.insert(contract_addr.to_felt());
                                    ClassUpdateItem::DeployedContract(class_hash.to_felt())
                                } else {
                                    ClassUpdateItem::ReplacedClass(class_hash.to_felt())
                                };

                                Ok((contract_addr.to_felt(), entry))
                            })
                            .collect::<anyhow::Result<_>>()?,
                        storage_diffs: state_diff
                            .storage
                            .into_iter()
                            .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
                            .collect(),
                        declared_classes: declared_class
                            .iter()
                            .map(|class| {
                                (
                                    *class.class_hash(),
                                    class
                                        .as_sierra()
                                        .map(|class| DeclaredClassCompiledClass::Sierra(class.info.compiled_class_hash))
                                        .unwrap_or(DeclaredClassCompiledClass::Legacy),
                                )
                            })
                            .collect(),
                    },
                    declared_class,
                    arrived_at: additional_info.arrived_at,
                    paid_fee_on_l1,
                })
            }
        }

        let backend = self.backend.clone();
        global_spawn_rayon_task(move || {
            backend
                .write_access()
                .append_to_preconfirmed(&executed, /* candidates */ [])
                .context("Appending to preconfirmed block")
        })
        .await?;

        let stats = mem::take(&mut batch.stats);
        if stats.n_added_to_block > 0 {
            tracing::info!(
                "🧮 Executed and added {} transaction(s) to the preconfirmed block at height {} - {:.3?}",
                stats.n_added_to_block,
                self.block_number,
                stats.exec_duration,
            );
            tracing::debug!("Tick stats {:?}", stats);
        }
        Ok(())
    }
}

/// Little state machine that helps us following the state transitions the executor thread sends us.
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
    current_state: Option<TaskState>,
    metrics: Arc<BlockProductionMetrics>,
    state_notifications: Option<mpsc::UnboundedSender<BlockProductionStateNotification>>,
    handle: BlockProductionHandle,
    executor_commands_recv: Option<mpsc::UnboundedReceiver<executor::ExecutorCommand>>,
    l1_client: Arc<dyn SettlementClient>,
    bypass_tx_input: Option<mpsc::Receiver<ValidatedTransaction>>,
    no_charge_fee: bool,
}

impl BlockProductionTask {
    pub fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_client: Arc<dyn SettlementClient>,
        no_charge_fee: bool,
    ) -> Self {
        let (sender, recv) = mpsc::unbounded_channel();
        let (bypass_input_sender, bypass_tx_input) = mpsc::channel(16);
        Self {
            backend: backend.clone(),
            mempool,
            current_state: None,
            metrics,
            handle: BlockProductionHandle::new(backend, sender, bypass_input_sender, no_charge_fee),
            state_notifications: None,
            executor_commands_recv: Some(recv),
            l1_client,
            bypass_tx_input: Some(bypass_tx_input),
            no_charge_fee,
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

    /// Prepares a PreconfirmedExecutedTransaction for re-execution by converting it to blockifier Transaction format.
    ///
    /// This function properly handles execution flags including charge_fee by converting through ValidatedTransaction.
    ///
    /// IMPORTANT: `PreconfirmedExecutedTransaction` doesn't store the original `charge_fee` value, so we use
    /// `charge_fee = true` as the default (matching `to_validated()`). This ensures re-execution matches the
    /// original execution, since transactions are typically executed with `charge_fee = true` during normal
    /// block production. If the original execution used `charge_fee = false`, the state diff would be different
    /// and this re-execution would fail anyway, which is expected behavior.
    fn prepare_preconfirmed_tx_for_reexecution(
        &self,
        preconfirmed_tx: &PreconfirmedExecutedTransaction,
        state_view: &MadaraStateView,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        // Convert PreconfirmedExecutedTransaction to ValidatedTransaction
        // Use the actual charge_fee value from configuration (charge_fee = !no_charge_fee)
        let mut validated_tx = preconfirmed_tx.to_validated();
        validated_tx.charge_fee = !self.no_charge_fee;

        // If declared_class is missing and transaction is Declare, fetch it from state_view
        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                validated_tx.declared_class = Some(
                    state_view
                        .get_class_info_and_compiled(declare_tx.class_hash())?
                        .with_context(|| format!("No class found for class_hash={:#x}", declare_tx.class_hash()))?,
                );
            }
        }

        // Use into_blockifier_for_sequencing which properly sets execution flags including charge_fee
        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for reexecution")?;

        Ok(blockifier_tx)
    }

    /// Helper function to get the hash of block_n-10 if it exists.
    fn wait_for_hash_of_block_min_10(
        backend: &Arc<MadaraBackend>,
        block_n: u64,
    ) -> anyhow::Result<Option<(u64, Felt)>> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else {
            return Ok(None);
        };

        if let Some(view) = backend.block_view_on_confirmed(block_n_min_10) {
            let block_hash = view.get_block_info().context("Getting block hash of block_n - 10")?.block_hash;
            Ok(Some((block_n_min_10, block_hash)))
        } else {
            // Block doesn't exist yet - this is fine, we'll skip it
            Ok(None)
        }
    }

    /// Re-executes all transactions in a PreconfirmedBlock to obtain BlockExecutionSummary.
    ///
    /// This function recreates the execution context and executes all transactions to get
    /// the bouncer_weights and state_diff needed for proper block closing.
    async fn reexecute_preconfirmed_block(
        &self,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
    ) -> anyhow::Result<BlockExecutionSummary> {
        // Get all executed transactions
        let executed_txs: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();

        // Get parent block state view
        let parent_state_view = preconfirmed_view.state_view_on_parent();

        // Convert transactions to blockifier format
        let blockifier_txs: Vec<blockifier::transaction::transaction_execution::Transaction> = executed_txs
            .iter()
            .map(|preconfirmed_tx| self.prepare_preconfirmed_tx_for_reexecution(preconfirmed_tx, &parent_state_view))
            .collect::<Result<Vec<_>, _>>()
            .context("Converting preconfirmed transactions to blockifier format")?;

        // Create BlockExecutionContext from PreconfirmedBlock header (preserving exact saved values)
        let header = &preconfirmed_view.block().header;
        let exec_ctx = BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        // Create LayeredStateAdapter
        let state_adaptor =
            LayeredStateAdapter::new(self.backend.clone()).context("Creating LayeredStateAdapter for re-execution")?;

        // Create TransactionExecutor with block_n-10 handling
        let mut executor =
            crate::util::create_executor_with_block_n_min_10(&self.backend, &exec_ctx, state_adaptor, |block_n| {
                Self::wait_for_hash_of_block_min_10(&self.backend, block_n)
            })
            .context("Creating TransactionExecutor for re-execution")?;

        // Execute all transactions
        let execution_results = executor.execute_txs(&blockifier_txs, /* execution_deadline */ None);

        // Check for execution errors (though we don't need to process results, we should check for panics)
        for result in &execution_results {
            if let Err(err) = result {
                tracing::warn!("Transaction execution error during re-execution: {err:?}");
            }
        }

        // Call finalize() to get BlockExecutionSummary
        let block_exec_summary = executor.finalize().context("Finalizing executor to get BlockExecutionSummary")?;

        Ok(block_exec_summary)
    }

    /// Closes the last pending block store in db (if any).
    ///
    /// Re-executes transactions if the block is not empty to obtain bouncer_weights and state_diff
    /// before closing the block. This ensures correctness on restart.
    async fn close_pending_block_if_exists(&mut self) -> anyhow::Result<()> {
        if !self.backend.has_preconfirmed_block() {
            return Ok(());
        }

        tracing::debug!("Close pending block on startup.");

        let preconfirmed_view = self.backend.block_view_on_preconfirmed().context("Getting preconfirmed block view")?;

        let block_number = preconfirmed_view.block_number();
        let n_txs = preconfirmed_view.num_executed_transactions();

        // If block is empty, close directly without re-execution
        if n_txs == 0 {
            tracing::debug!("Closing empty preconfirmed block on startup.");
            let backend = self.backend.clone();
            global_spawn_rayon_task(move || {
                backend
                    .write_access()
                    .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, None)
                    .context("Closing empty preconfirmed block on startup")
            })
            .await?;
            return Ok(());
        }

        tracing::info!(
            "Re-executing {} transaction(s) in preconfirmed block #{} to obtain bouncer_weights and state_diff",
            n_txs,
            block_number
        );

        // Re-execute transactions to get BlockExecutionSummary
        let block_exec_summary = self
            .reexecute_preconfirmed_block(&preconfirmed_view)
            .await
            .context("Re-executing preconfirmed block to get execution summary")?;

        // Extract consumed L1 nonces from transactions
        let consumed_core_contract_nonces: HashSet<u64> = preconfirmed_view
            .borrow_content()
            .executed_transactions()
            .filter_map(|tx| tx.transaction.transaction.as_l1_handler().map(|l1_tx| l1_tx.nonce))
            .collect();

        let backend = self.backend.clone();
        global_spawn_rayon_task(move || {
            // Remove consumed L1 to L2 message nonces
            for l1_nonce in consumed_core_contract_nonces {
                backend
                    .remove_pending_message_to_l2(l1_nonce)
                    .context("Removing pending message to l2 from database")?;
            }

            // Save bouncer weights
            backend
                .write_access()
                .write_bouncer_weights(block_number, &block_exec_summary.bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;

            // Convert state_diff and close block
            let state_diff: mp_state_update::StateDiff = block_exec_summary.state_diff.into();
            backend
                .write_access()
                .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, Some(state_diff))
                .context("Closing preconfirmed block on startup")?;

            anyhow::Ok(())
        })
        .await?;

        tracing::info!("✅ Closed preconfirmed block #{} with {} transactions on startup", block_number, n_txs);

        Ok(())
    }

    /// Handles the state machine and its transitions.
    async fn process_reply(&mut self, reply: ExecutorMessage) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { exec_ctx } => {
                tracing::debug!("Received ExecutorMessage::StartNewBlock block_n={}", exec_ctx.block_number);
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::NotExecuting { latest_block_n } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let new_block_n = latest_block_n.map(|n| n + 1).unwrap_or(/* genesis */ 0);
                if new_block_n != exec_ctx.block_number {
                    anyhow::bail!(
                        "Received new block_n={} from executor, expected block_n={}",
                        exec_ctx.block_number,
                        new_block_n
                    )
                }

                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header()))
                })
                .await?;

                self.current_state =
                    Some(TaskState::Executing(CurrentBlockState::new(self.backend.clone(), new_block_n)));
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

                state.append_batch(batch_execution_result).await?;

                self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
            }
            ExecutorMessage::EndBlock(block_exec_summary) => {
                tracing::debug!("Received ExecutorMessage::EndBlock");
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };

                tracing::debug!("Close and save block block_n={}", state.block_number);
                let start_time = Instant::now();

                let n_txs = self
                    .backend
                    .block_view_on_preconfirmed()
                    .context("No current pre-confirmed block")?
                    .num_executed_transactions();

                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    for l1_nonce in state.consumed_core_contract_nonces {
                        // This ensures we remove the nonces for rejected L1 to L2 message transactions. This avoids us from reprocessing them on restart.
                        backend
                            .remove_pending_message_to_l2(l1_nonce)
                            .context("Removing pending message to l2 from database")?;
                    }

                    backend
                        .write_access()
                        .write_bouncer_weights(state.block_number, &block_exec_summary.bouncer_weights)
                        .context("Saving Bouncer Weights for SNOS")?;

                    let state_diff: mp_state_update::StateDiff = block_exec_summary.state_diff.into();
                    backend
                        .write_access()
                        .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, Some(state_diff))
                        .context("Closing block")?;
                    anyhow::Ok(())
                })
                .await?;

                let time_to_close = start_time.elapsed();
                tracing::info!(
                    "⛏️  Closed block #{} with {n_txs} transactions - {time_to_close:?}",
                    state.block_number
                );

                // Record metrics
                let attributes = [
                    KeyValue::new("transactions_added", n_txs.to_string()),
                    KeyValue::new("closing_time", time_to_close.as_secs_f32().to_string()),
                ];

                self.metrics.block_counter.add(1, &[]);
                self.metrics.block_gauge.record(state.block_number, &attributes);
                self.metrics.transaction_counter.add(n_txs as u64, &[]);

                self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(state.block_number) });
                self.send_state_notification(BlockProductionStateNotification::ClosedBlock);
            }
        }

        Ok(())
    }

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_pending_block_if_exists().await.context("Cannot close pending block on startup")?;

        // initial state
        let latest_block_n = self.backend.latest_confirmed_block_n();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n });

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
    use blockifier::bouncer::{BouncerConfig, BouncerWeights};
    use mc_db::MadaraBackend;
    use mc_devnet::{
        Call, ChainGenesisDescription, DevnetKeys, DevnetPredeployedContract, Multicall, Selector, UDC_CONTRACT_ADDRESS,
    };
    use mc_mempool::{Mempool, MempoolConfig};
    use mc_settlement_client::L1ClientMock;
    use mc_submit_tx::{SubmitTransaction, TransactionValidator, TransactionValidatorConfig};
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use mp_receipt::{Event, ExecutionResult};
    use mp_rpc::v0_9_0::{
        BroadcastedDeclareTxn, BroadcastedDeclareTxnV3, BroadcastedInvokeTxn, BroadcastedTxn, ClassAndTxnHash, DaMode,
        InvokeTxnV3, ResourceBounds, ResourceBoundsMapping,
    };
    use mp_transactions::compute_hash::calculate_contract_address;
    use mp_transactions::IntoStarknetApiExt;
    use mp_transactions::{L1HandlerTransaction, L1HandlerTransactionWithFee, Transaction};
    use mp_utils::service::ServiceContext;
    use mp_utils::AbortOnDrop;
    use starknet_core::utils::get_selector_from_name;
    use starknet_types_core::felt::Felt;
    use std::{sync::Arc, time::Duration};

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
        BouncerWeights { sierra_gas: starknet_api::execution_resources::GasAmount(1000000), ..BouncerWeights::max() }
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
                false, /* no_charge_fee = false */
            )
        }
    }

    #[rstest::fixture]
    pub async fn devnet_setup(
        #[default(Duration::from_secs(30))] block_time: Duration,
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
                bouncer_config: BouncerConfig {
                    block_max_capacity: bouncer_weights,
                    builtin_weights: Default::default(),
                    blake_weight: Default::default(),
                },
                ..ChainConfig::madara_devnet()
            })
        } else {
            Arc::new(ChainConfig { block_time, ..ChainConfig::madara_devnet() })
        };

        let backend = MadaraBackend::open_for_testing(Arc::clone(&chain_config));
        backend.set_l1_gas_quote_for_testing();
        genesis.build_and_store(&backend).await.unwrap();

        let mempool = Arc::new(Mempool::new(Arc::clone(&backend), MempoolConfig::default()));
        let tx_validator = Arc::new(TransactionValidator::new(
            Arc::clone(&mempool) as _,
            Arc::clone(&backend),
            TransactionValidatorConfig::default(), /* disable_fee = false, disable_validation = false */
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
                l1_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 10000 },
                l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
                l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
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
                l2_gas: ResourceBounds { max_amount: 10000000000, max_price_per_unit: 10000000 },
                l1_data_gas: ResourceBounds { max_amount: 60000, max_price_per_unit: 60000 },
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

    /// Test that `close_pending_block_if_exists` correctly re-executes transactions
    /// and produces the same global state root and state diff as normal block closing.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(100))]
    #[tokio::test]
    async fn test_close_pending_block_reexecution_matches_normal_closing(
        #[future]
        #[from(devnet_setup)]
        original_devnet_setup: DevnetSetup,
        #[future]
        #[from(devnet_setup)]
        restart_devnet_setup: DevnetSetup,
    ) {
        // used for phase 1, where we close the block and note down its
        // global_state_root, state_diff, and header info
        let mut original_devnet_setup = original_devnet_setup.await;

        // use for phase 2, where we compare the state of the block after re-execution with the state of the block before re-execution
        let mut restart_devnet_setup = restart_devnet_setup.await;

        // --------------------------------------------------------------
        // | PHASE 1: Close the block and note down its state.          |
        // --------------------------------------------------------------

        // Step 1: Create a block normally with transactions in the original backend
        assert!(original_devnet_setup.mempool.is_empty().await);

        // Add various transaction types to mempool to test re-execution handles all types correctly
        // All transactions will be in a single block

        // 1. Declare a contract
        let declare_res = sign_and_add_declare_tx(
            &original_devnet_setup.contracts.0[0],
            &original_devnet_setup.backend,
            &original_devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;

        // 2. Deploy contract through UDC
        let (contract_address, deploy_tx) = make_udc_call(
            &original_devnet_setup.contracts.0[0],
            &original_devnet_setup.backend,
            /* nonce */ Felt::ONE,
            declare_res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        original_devnet_setup.tx_validator.submit_invoke_transaction(deploy_tx).await.unwrap();

        // 3. Invoke transaction
        sign_and_add_invoke_tx(
            &original_devnet_setup.contracts.0[0],
            &original_devnet_setup.contracts.0[1],
            &original_devnet_setup.backend,
            &original_devnet_setup.tx_validator,
            Felt::TWO, // nonce after declare (ZERO) and deploy (ONE)
        )
        .await;

        // 4. Declare transaction (for a different contract)
        sign_and_add_declare_tx(
            &original_devnet_setup.contracts.0[2],
            &original_devnet_setup.backend,
            &original_devnet_setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 5. Another invoke transaction
        sign_and_add_invoke_tx(
            &original_devnet_setup.contracts.0[1],
            &original_devnet_setup.contracts.0[3],
            &original_devnet_setup.backend,
            &original_devnet_setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 6. Add L1 handler transaction
        let paid_fee_on_l1 = 128328u128;
        original_devnet_setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
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
            paid_fee_on_l1,
        ));

        assert!(!original_devnet_setup.mempool.is_empty().await);

        // Run block production to create and close a block with all transactions
        let mut block_production_task = original_devnet_setup.block_prod_task();
        let mut notifications = block_production_task.subscribe_state_notifications();
        let control = block_production_task.handle();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // Wait for batch to be executed
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Manually close the block
        control.close_block().await.unwrap();
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

        // Step 2: Capture global_state_root, state_diff, and header info from closed block
        let block_number = original_devnet_setup.backend.latest_confirmed_block_n().unwrap();
        let original_block = original_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
        let original_block_info = original_block.get_block_info().unwrap();
        let expected_global_state_root = original_block_info.header.global_state_root;
        let expected_state_diff = original_block.get_state_diff().unwrap();
        let executed_transactions = original_block.get_executed_transactions(..).unwrap();

        // --------------------------------------------------------------
        // | PHASE 2: Re-execute the block and note down its state.    |
        // --------------------------------------------------------------
        //
        // We'll add them in the same order using the same helper functions
        // All transactions will be in a single block
        // This ensures they're executed in the same context (clean genesis state)
        assert!(restart_devnet_setup.mempool.is_empty().await);

        // 1. Declare a contract
        let restart_declare_res = sign_and_add_declare_tx(
            &restart_devnet_setup.contracts.0[0],
            &restart_devnet_setup.backend,
            &restart_devnet_setup.tx_validator,
            Felt::ZERO,
        )
        .await;

        // 2. Deploy contract through UDC
        let (restart_contract_address, restart_deploy_tx) = make_udc_call(
            &restart_devnet_setup.contracts.0[0],
            &restart_devnet_setup.backend,
            /* nonce */ Felt::ONE,
            restart_declare_res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        restart_devnet_setup.tx_validator.submit_invoke_transaction(restart_deploy_tx).await.unwrap();

        // 3. Invoke transaction
        sign_and_add_invoke_tx(
            &restart_devnet_setup.contracts.0[0],
            &restart_devnet_setup.contracts.0[1],
            &restart_devnet_setup.backend,
            &restart_devnet_setup.tx_validator,
            Felt::TWO, // nonce after declare (ZERO) and deploy (ONE)
        )
        .await;

        // 4. Declare transaction (for a different contract)
        sign_and_add_declare_tx(
            &restart_devnet_setup.contracts.0[2],
            &restart_devnet_setup.backend,
            &restart_devnet_setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 5. Another invoke transaction
        sign_and_add_invoke_tx(
            &restart_devnet_setup.contracts.0[1],
            &restart_devnet_setup.contracts.0[3],
            &restart_devnet_setup.backend,
            &restart_devnet_setup.tx_validator,
            Felt::ZERO, // Different account, so nonce starts at ZERO
        )
        .await;

        // 6. Add the same L1 handler transaction
        restart_devnet_setup.l1_client.add_tx(L1HandlerTransactionWithFee::new(
            L1HandlerTransaction {
                version: Felt::ZERO,
                nonce: 55, // core contract nonce
                contract_address: restart_contract_address,
                entry_point_selector: get_selector_from_name("l1_handler_entrypoint").unwrap(),
                calldata: vec![
                    /* from_address */ Felt::THREE,
                    /* arg1 */ Felt::ONE,
                    /* arg2 */ Felt::TWO,
                ]
                .into(),
            },
            paid_fee_on_l1, // Use the same paid_fee_on_l1 value
        ));

        assert!(!restart_devnet_setup.mempool.is_empty().await);

        // Step 4: Run block production to execute transactions and add them to preconfirmed block
        // Use a very long block_time to prevent auto-closing, then stop manually after batch execution
        let mut restart_block_production_task = restart_devnet_setup.block_prod_task();
        let mut restart_notifications = restart_block_production_task.subscribe_state_notifications();
        let restart_task = AbortOnDrop::spawn(async move {
            restart_block_production_task.run(ServiceContext::new_for_testing()).await.unwrap()
        });

        // Wait for batch to be executed (transactions added to preconfirmed block)
        assert_eq!(restart_notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        // Stop the task before it closes the block (drop the AbortOnDrop which will abort the task)
        drop(restart_task);

        // Give it a moment to finish current operations
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify preconfirmed block exists with transactions and no confirmed blocks yet
        assert!(restart_devnet_setup.backend.has_preconfirmed_block());
        assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(0));

        let preconfirmed_view = restart_devnet_setup.backend.block_view_on_preconfirmed().unwrap();
        assert_eq!(preconfirmed_view.num_executed_transactions(), executed_transactions.len());

        let restart_preconfirmed_block = preconfirmed_view.block();

        // adding some delay to see if block_timestamp would differ in the reexecution or not
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Step 5: Now call close_pending_block_if_exists to re-execute and close
        let mut reexec_block_production_task = restart_devnet_setup.block_prod_task();
        reexec_block_production_task.close_pending_block_if_exists().await.unwrap();

        // Step 6: Verify results match
        assert!(!restart_devnet_setup.backend.has_preconfirmed_block());
        assert_eq!(restart_devnet_setup.backend.latest_confirmed_block_n(), Some(block_number));

        let reexecuted_block_info =
            restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap().get_block_info().unwrap();

        // Verify the header fields match the pre-execution pre-confirmed block's header
        assert_eq!(restart_preconfirmed_block.header.block_timestamp, reexecuted_block_info.header.block_timestamp);
        assert_eq!(restart_preconfirmed_block.header.protocol_version, reexecuted_block_info.header.protocol_version);
        assert_eq!(restart_preconfirmed_block.header.l1_da_mode, reexecuted_block_info.header.l1_da_mode);
        assert_eq!(restart_preconfirmed_block.header.gas_prices, reexecuted_block_info.header.gas_prices);
        assert_eq!(restart_preconfirmed_block.header.sequencer_address, reexecuted_block_info.header.sequencer_address);
        assert_eq!(restart_preconfirmed_block.header.block_number, reexecuted_block_info.header.block_number);

        let reexecuted_block = restart_devnet_setup.backend.block_view_on_confirmed(block_number).unwrap();
        let reexecuted_block_info = reexecuted_block.get_block_info().unwrap();
        let actual_global_state_root = reexecuted_block_info.header.global_state_root;
        let mut actual_state_diff = reexecuted_block.get_state_diff().unwrap();
        let mut expected_state_diff_sorted = expected_state_diff.clone();

        // Sort both state diffs to normalize ordering before comparison
        actual_state_diff.sort();
        expected_state_diff_sorted.sort();

        // Verify global state root matches
        assert_eq!(
            actual_global_state_root, expected_global_state_root,
            "Global state root should match between normal execution and re-execution"
        );

        // Verify state diff matches (after sorting to ignore ordering differences)
        assert_eq!(
            actual_state_diff, expected_state_diff_sorted,
            "State diff should match between normal execution and re-execution (values are the same, only order may differ)"
        );

        // Verify transactions match
        let reexecuted_transactions = reexecuted_block.get_executed_transactions(..).unwrap();
        assert_eq!(reexecuted_transactions, executed_transactions, "Transactions should match");
    }

    // This test makes sure that the pending tick closes the block
    // if the bouncer capacity is reached
    #[ignore] // FIXME: this test is complicated by the fact validation / actual execution fee may differ a bit. Ignore for now.
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_bouncer_cap_reached_closes_block(
        #[future]
        // Use a very very long block time (longer than the test timeout).
        #[with(Duration::from_secs(10000000), true)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

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

        let mut block_production_task = devnet_setup.block_prod_task();
        // The BouncerConfig is set up with amounts (100000) that should limit
        // the block size in a way that the pending tick on this task
        // closes the block
        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        tokio::time::sleep(Duration::from_secs(5)).await;

        tracing::debug!("{:?}", devnet_setup.backend.block_view_on_latest().map(|l| l.get_executed_transactions(..)));
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        let closed_1 = devnet_setup.backend.block_view_on_confirmed(1).unwrap();
        let closed_2 = devnet_setup.backend.block_view_on_confirmed(2).unwrap();
        let preconfirmed_3 = devnet_setup.backend.block_view_on_preconfirmed().unwrap();
        assert_eq!(preconfirmed_3.block_number(), 3);
        assert_eq!(closed_1.get_executed_transactions(..).unwrap().len(), 1);
        // rolled over to next block.
        assert_eq!(closed_2.get_executed_transactions(..).unwrap().len(), 1);
        // rolled over to next block.
        // last block should not be closed though.
        assert_eq!(preconfirmed_3.get_executed_transactions(..).len(), 1);
        assert!(devnet_setup.mempool.is_empty().await);
    }

    // This test makes sure that the block time tick correctly
    // adds the transaction to the pending block, closes it
    // and creates a new empty pending block
    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_block_time_tick_closes_block(
        #[future]
        #[with(Duration::from_secs(2), true)]
        devnet_setup: DevnetSetup,
    ) {
        let mut devnet_setup = devnet_setup.await;

        let mut block_production_task = devnet_setup.block_prod_task();

        let mut notifications = block_production_task.subscribe_state_notifications();
        let _task =
            AbortOnDrop::spawn(
                async move { block_production_task.run(ServiceContext::new_for_testing()).await.unwrap() },
            );

        // The block should be closed after 3s.
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

        let view = devnet_setup.backend.block_view_on_last_confirmed().unwrap();

        assert_eq!(view.block_number(), 1);
        assert_eq!(view.get_executed_transactions(..).unwrap(), []);
    }

    #[rstest::rstest]
    #[timeout(Duration::from_secs(30))]
    #[tokio::test]
    async fn test_l1_handler_tx(
        #[future]
        #[with(Duration::from_secs(3000000000), false)]
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

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        assert_eq!(
            devnet_setup
                .backend
                .block_view_on_preconfirmed()
                .unwrap()
                .get_executed_transaction(0)
                .unwrap()
                .receipt
                .execution_result(),
            ExecutionResult::Succeeded
        );
        control.close_block().await.unwrap();
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

        // Deploy contract through UDC.

        let (contract_address, tx) = make_udc_call(
            &devnet_setup.contracts.0[0],
            &devnet_setup.backend,
            /* nonce */ Felt::ONE,
            res.class_hash,
            /* calldata (pubkey) */ &[Felt::TWO],
        );
        devnet_setup.tx_validator.submit_invoke_transaction(tx).await.unwrap();

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        assert_eq!(
            devnet_setup
                .backend
                .block_view_on_preconfirmed()
                .unwrap()
                .get_executed_transaction(0)
                .unwrap()
                .receipt
                .execution_result(),
            ExecutionResult::Succeeded
        );

        control.close_block().await.unwrap();
        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::ClosedBlock);

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

        assert_eq!(notifications.recv().await.unwrap(), BlockProductionStateNotification::BatchExecuted);

        let receipt =
            devnet_setup.backend.block_view_on_preconfirmed().unwrap().get_executed_transaction(0).unwrap().receipt;
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
