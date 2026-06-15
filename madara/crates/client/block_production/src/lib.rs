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
//! - [`EndFinalBlock`]: sent during graceful shutdown when batch channel closes. Contains
//!   `Some(summary)` to close an existing block, or `None` to signal completion without a block.
//!
//! ## Pending Phase
//!
//! The [`PendingBlockState`] is primarily kept in RAM but is also flushed to the database as a
//! **pre-confirmed block**. This ensures that if the node crashes or restarts during block
//! production, we can recover the work done so far.
//!
//! Currently, we flush the pending block state to the database whenever a new batch of transactions
//! is executed (`BatchExecuted` message). This persistence allows us to recover the pre-confirmed
//! block upon restart.
//!
//! ### Restart Recovery
//!
//! When Madara starts, it checks for an existing pre-confirmed block. If found:
//! 1. It loads the saved **runtime execution configuration** (ChainConfig, VersionedConstants, etc.)
//!    to ensure re-execution uses the exact same parameters as the original execution.
//! 2. It **re-executes** all transactions in the pre-confirmed block. This is necessary because
//!    intermediate execution artifacts (like bouncer weights and state diffs) are not fully persisted.
//! 3. It closes the block immediately, effectively resuming the chain from where it left off.
//!
//! This mechanism guarantees consistency (e.g., transaction receipts match exactly) even if the
//! node's configuration changes between restarts (e.g., toggling fee charging).
//!
//! ## Graceful Shutdown and Error Handling
//!
//! The [`BlockProductionTask::run`] method implements graceful shutdown and error handling for
//! batcher and executor tasks. The main loop tracks completion of both tasks, which only complete
//! during shutdown scenarios (cancellation, error, or panic).
//!
//! ### Graceful Shutdown
//!
//! When a cancellation signal is received:
//! 1. The batcher detects cancellation and exits gracefully, closing the `send_batch` channel
//! 2. The executor detects the channel closure and finalizes any open block
//! 3. The executor sends an `EndFinalBlock` message (shutdown-specific) and then completes
//! 4. The main loop processes the `EndFinalBlock`, closes the block, and exits when both tasks complete
//!
//! ### Batcher Panic/Error
//!
//! If the batcher encounters an error or panics:
//! - **With preconfirmed block**: The error is saved and graceful shutdown is attempted. The batcher
//!   closes the channel, executor closes the block, and shutdown completes with the saved error.
//! - **Without preconfirmed block**: The error is returned immediately (no need to wait for executor).
//!
//! ### Executor Panic
//!
//! If the executor thread panics:
//! - The panic is caught and propagated via the `stop` channel
//! - The main loop resumes the panic, causing the block to remain preconfirmed
//! - The preconfirmed block will be handled on restart
//!
//! The loop exits when:
//! - Batcher completed AND `EndFinalBlock` was processed → returns `Ok(())` or saved batcher error
//!
//! [mempool]: mc_mempool
//! [`StartNewBlock`]: ExecutorMessage::StartNewBlock
//! [`BatchExecuted`]: ExecutorMessage::BatchExecuted
//! [`EndBlock`]: ExecutorMessage::EndBlock
//! [`EndFinalBlock`]: ExecutorMessage::EndFinalBlock
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
use mp_chain_config::RuntimeExecutionConfig;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{ClassUpdateItem, DeclaredClassCompiledClass, TransactionStateUpdate};
use mp_transactions::validated::ValidatedTransaction;
use mp_transactions::TransactionWithHash;
use mp_utils::rayon::global_spawn_rayon_task;
use mp_utils::service::ServiceContext;
use mp_utils::AbortOnDrop;
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
    /// Track when block production started for metrics
    pub block_start_time: Instant,
    /// Accumulated execution stats across all batches for this block
    pub accumulated_stats: util::ExecutionStats,
}

impl CurrentBlockState {
    pub fn new(backend: Arc<MadaraBackend>, block_number: u64) -> Self {
        Self {
            backend,
            block_number,
            consumed_core_contract_nonces: Default::default(),
            deployed_contracts: Default::default(),
            block_start_time: Instant::now(),
            accumulated_stats: Default::default(),
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
                                        .and_then(|class| {
                                            // Use canonical hash (v2 if present, else v1)
                                            let hash =
                                                class.info.compiled_class_hash_v2.or(class.info.compiled_class_hash)?;
                                            Some(DeclaredClassCompiledClass::Sierra(hash))
                                        })
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
    discard_preconfirmed_on_startup: bool,
}

impl BlockProductionTask {
    /// Creates a new BlockProductionTask.
    ///
    /// # Parameters
    ///
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
        no_charge_fee: bool,
        discard_preconfirmed_on_startup: bool,
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
            discard_preconfirmed_on_startup,
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

    /// Prepares a PreconfirmedExecutedTransaction for re-execution by converting it to blockifier format.
    ///
    /// This function converts a `PreconfirmedExecutedTransaction` (stored in the database) back into a
    /// blockifier transaction format that can be re-executed. It handles all the necessary conversions
    /// and ensures execution flags are properly set.
    ///
    /// # Process
    ///
    /// 1. Converts `PreconfirmedExecutedTransaction` to `ValidatedTransaction` using `to_validated()`
    /// 2. Sets `charge_fee` based on the `no_charge_fee` configuration (`charge_fee = !no_charge_fee`)
    /// 3. Fetches `declared_class` from state if missing (for Declare transactions)
    /// 4. Converts to blockifier format using `into_blockifier_for_sequencing()` which properly applies execution flags
    ///
    /// # Important Notes
    ///
    /// - The `charge_fee` flag is determined by `self.no_charge_fee` configuration. Note that `new()` is
    ///   called every time Madara starts, so there is no guarantee that the `no_charge_fee` value matches
    ///   the value used during original execution. This is a limitation that should be addressed by storing
    ///   execution configuration in the database (see TODO in `new()`).
    /// - For L1 handler transactions, `paid_fee_on_l1` is preserved from `PreconfirmedExecutedTransaction`
    ///   (stored during `append_batch`) and used during conversion via `to_validated()`
    /// - Declare transactions may need their `declared_class` fetched from state if not already stored
    /// - The conversion uses `into_blockifier_for_sequencing()` which properly sets all execution flags
    ///   including `charge_fee`, `validate`, and `only_query`
    fn prepare_preconfirmed_tx_for_reexecution(
        &self,
        preconfirmed_tx: &PreconfirmedExecutedTransaction,
        state_view: &MadaraStateView,
        no_charge_fee: bool,
    ) -> anyhow::Result<blockifier::transaction::transaction_execution::Transaction> {
        // Convert PreconfirmedExecutedTransaction to ValidatedTransaction
        // Use the actual charge_fee value from configuration (charge_fee = !no_charge_fee)
        let mut validated_tx = preconfirmed_tx.to_validated();
        validated_tx.charge_fee = !no_charge_fee;

        // If declared_class is missing and transaction is Declare, fetch it from state_view
        // NOTE: For declare transactions in the preconfirmed block, declared_class MUST be stored
        // during append_batch. If it's None here, that indicates data corruption - we should panic.
        if validated_tx.declared_class.is_none() {
            if let Some(declare_tx) = validated_tx.transaction.as_declare() {
                // This should never happen for declare transactions in the preconfirmed block
                // If it does, it indicates missing data that should have been stored during original execution
                validated_tx.declared_class = Some(
                    state_view
                        .get_class_info_and_compiled(declare_tx.class_hash())
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "CRITICAL: Error fetching class for class_hash={:#x} in preconfirmed block. \
                                 This indicates data corruption - declared_class should have been stored during append_batch. Error: {}",
                                declare_tx.class_hash(),
                                e
                            )
                        })?
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "CRITICAL: Class not found for class_hash={:#x} in parent state view. \
                                 For declare transactions in the preconfirmed block, declared_class must be stored during append_batch.",
                                declare_tx.class_hash()
                            )
                        })?,
                );
            }
        }

        // Use into_blockifier_for_sequencing which properly sets execution flags including charge_fee
        let (blockifier_tx, _, _) = validated_tx
            .into_blockifier_for_sequencing()
            .context("Error converting validated transaction to blockifier format for reexecution")?;

        Ok(blockifier_tx)
    }

    /// Helper function to close a preconfirmed block with the given state_diff and bouncer weights.
    /// This is used both during normal block closing (EndBlock case) and during restart recovery.
    /// Returns the result including timing information from the DB layer.
    async fn close_preconfirmed_block_with_state_diff(
        backend: Arc<MadaraBackend>,
        block_number: u64,
        consumed_core_contract_nonces: HashSet<u64>,
        bouncer_weights: &blockifier::bouncer::BouncerWeights,
        state_diff: mp_state_update::StateDiff,
    ) -> anyhow::Result<mc_db::AddFullBlockResult> {
        // Copy bouncer_weights to move into the closure (BouncerWeights implements Copy)
        let bouncer_weights = *bouncer_weights;
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
                .write_bouncer_weights(block_number, &bouncer_weights)
                .context("Saving Bouncer Weights for SNOS")?;

            // Close the preconfirmed block with state_diff
            let result = backend
                .write_access()
                .close_preconfirmed(/* pre_v0_13_2_hash_override */ true, state_diff)
                .context("Closing preconfirmed block")?;

            anyhow::Ok(result)
        })
        .await
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
            // This should be unreachable - if we're here, something is wrong
            unreachable!("Block doesn't exist yet - this path should not be reachable")
        }
    }

    /// Re-executes all transactions in a PreconfirmedBlock to obtain BlockExecutionSummary.
    ///
    /// This function is called when Madara restarts with a preconfirmed block in the database.
    /// It recreates the execution context and re-executes all transactions to regenerate:
    /// - `bouncer_weights`: Resource usage metrics required for block finalization
    /// - `state_diff`: Aggregated state changes needed for block closing
    ///
    /// # Process
    ///
    /// 1. Retrieves all executed transactions from the preconfirmed block
    /// 2. Converts them to blockifier format using `prepare_preconfirmed_tx_for_reexecution()`
    /// 3. Creates `BlockExecutionContext` from the preconfirmed block's header (preserving timestamp, gas_prices, etc.)
    /// 4. Sets up `LayeredStateAdapter` for state access
    /// 5. Creates `TransactionExecutor` with proper `block_n-10` state diff handling (Starknet protocol requirement)
    /// 6. Executes all transactions and calls `finalize()` to get `BlockExecutionSummary`
    ///
    /// # Important Notes
    ///
    /// - The execution context uses the exact header values from the preconfirmed block (timestamp, gas_prices, etc.)
    /// - This ensures re-execution produces the same results as the original execution
    /// - The `block_n-10` state diff entry is set on the `0x1` contract address for protocol compliance
    async fn reexecute_preconfirmed_block(
        &self,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        saved_chain_config: Option<&Arc<mp_chain_config::ChainConfig>>,
        saved_no_charge_fee: bool,
    ) -> anyhow::Result<BlockExecutionSummary> {
        // Get all executed transactions
        let executed_txs: Vec<_> = preconfirmed_view.borrow_content().executed_transactions().cloned().collect();

        // Get parent block state view
        let parent_state_view = preconfirmed_view.state_view_on_parent();

        // Convert transactions to blockifier format
        // Note: saved_no_charge_fee is passed here to ensure re-execution uses the saved value
        let blockifier_txs: Vec<blockifier::transaction::transaction_execution::Transaction> = executed_txs
            .iter()
            .map(|preconfirmed_tx| {
                self.prepare_preconfirmed_tx_for_reexecution(preconfirmed_tx, &parent_state_view, saved_no_charge_fee)
            })
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
        let state_adapter =
            LayeredStateAdapter::new(self.backend.clone()).context("Creating LayeredStateAdapter for re-execution")?;

        // Create TransactionExecutor with block_n-10 handling
        // Use saved configs if available, otherwise use current backend configs
        let custom_chain_config = saved_chain_config;

        let mut executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state_adapter,
            |block_n| Self::wait_for_hash_of_block_min_10(&self.backend, block_n),
            custom_chain_config, // Use saved chain_config if available (re-execution)
        )
        .context("Creating TransactionExecutor for re-execution")?;

        // Execute all transactions
        let execution_results = executor.execute_txs(&blockifier_txs, /* execution_deadline */ None);

        // Verify that re-execution produces matching receipts
        for (i, (result, preconfirmed_tx)) in execution_results.iter().zip(executed_txs.iter()).enumerate() {
            match result {
                Ok((exec_info, _state_maps)) => {
                    // Convert execution info to receipt
                    let reexecuted_receipt = from_blockifier_execution_info(exec_info, &blockifier_txs[i]);

                    // Compare receipts - they should match exactly
                    assert_eq!(
                        reexecuted_receipt.transaction_hash(),
                        preconfirmed_tx.transaction.receipt.transaction_hash(),
                        "Re-execution produced different receipt for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );

                    assert_eq!(
                        reexecuted_receipt,
                        preconfirmed_tx.transaction.receipt,
                        "Re-execution produced different receipt content for transaction {} (hash: {:#x})",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
                Err(err) => {
                    tracing::warn!("Transaction execution error during re-execution: {err:?}");
                    // If execution failed, we can't compare receipts, but this is unexpected
                    anyhow::bail!(
                        "Transaction {} (hash: {:#x}) failed during re-execution: {err:?}",
                        i,
                        preconfirmed_tx.transaction.receipt.transaction_hash()
                    );
                }
            }
        }

        // Call finalize() to get BlockExecutionSummary
        let block_exec_summary = executor.finalize().context("Finalizing executor to get BlockExecutionSummary")?;

        Ok(block_exec_summary)
    }

    /// Saves current runtime config for future restarts.
    fn save_current_runtime_exec_config(&self) -> anyhow::Result<()> {
        let current_chain_config = self.backend.chain_config();
        let current_exec_constants = current_chain_config
            .exec_constants_by_protocol_version(current_chain_config.latest_protocol_version)
            .context("Failed to resolve execution constants for latest protocol version")?;

        let runtime_config = RuntimeExecutionConfig::from_current_config(
            current_chain_config,
            current_exec_constants,
            self.no_charge_fee,
        )
        .context("Failed to create runtime execution config")?;

        self.backend
            .write_access()
            .write_runtime_exec_config(&runtime_config)
            .context("Saving runtime execution config")?;

        Ok(())
    }

    /// Closes the last preconfirmed block stored in the database (if any).
    ///
    /// This function is called when Madara restarts and finds a preconfirmed block in the database.
    /// It handles closing the block properly by re-executing transactions to regenerate execution context.
    ///
    /// # Process
    ///
    /// 1. Checks if a preconfirmed block exists.
    /// 2. Re-executes transactions to obtain `bouncer_weights` and `state_diff`.
    /// 3. Extracts L1 handler nonces and cleans up L1-L2 message nonces.
    /// 4. Saves bouncer weights and closes the block.
    /// 5. Updates runtime config for future blocks.
    ///
    /// Note: Re-execution uses saved config values (e.g. `no_charge_fee`) to ensure consistency with original execution.
    /// Runtime config is always saved for persistence.
    async fn close_preconfirmed_block_if_exists(&mut self) -> anyhow::Result<()> {
        if !self.backend.has_preconfirmed_block() {
            // Even if there's no preconfirmed block, save the current runtime exec config
            // This ensures the config is persisted for future restarts
            self.save_current_runtime_exec_config()?;
            return Ok(());
        }

        if self.discard_preconfirmed_on_startup {
            let preconfirmed_view =
                self.backend.block_view_on_preconfirmed().context("Getting preconfirmed block view")?;
            let block_number = preconfirmed_view.block_number();
            let n_txs = preconfirmed_view.num_executed_transactions();
            let tx_hashes: Vec<_> = preconfirmed_view
                .get_block_info()
                .tx_hashes
                .into_iter()
                .map(|tx_hash| format!("{tx_hash:#x}"))
                .collect();

            tracing::warn!(
                discarded_transaction_hashes = ?tx_hashes,
                "Discarding preconfirmed block #{} with {} transactions on startup because discard_preconfirmed_on_startup is enabled; these transactions are permanently lost and will not be re-queued",
                block_number,
                n_txs
            );

            let backend = self.backend.clone();
            global_spawn_rayon_task(move || {
                backend.write_access().clear_preconfirmed().context("Discarding preconfirmed block on startup")
            })
            .await?;

            self.save_current_runtime_exec_config()
                .context("Saving runtime execution config after discarding preconfirmed block")?;

            tracing::info!("🧹 Discarded preconfirmed block #{} on startup", block_number);
            return Ok(());
        }

        tracing::debug!("Close preconfirmed block on startup.");

        let preconfirmed_view = self.backend.block_view_on_preconfirmed().context("Getting preconfirmed block view")?;

        let block_number = preconfirmed_view.block_number();
        let n_txs = preconfirmed_view.num_executed_transactions();

        tracing::debug!(
            "Re-executing {} transaction(s) in preconfirmed block #{} to obtain bouncer_weights and state_diff",
            n_txs,
            block_number
        );

        // Load saved runtime execution config
        let saved_config = self.backend.get_runtime_exec_config().context("Getting runtime execution config")?;

        // Extract saved values for re-execution without modifying self
        let (saved_chain_config, saved_no_charge_fee) = if let Some(config) = saved_config {
            (Some(Arc::new(config.chain_config)), config.no_charge_fee)
        } else {
            tracing::warn!("No saved runtime execution config found, using current configs (backward compatibility)");
            (None, self.no_charge_fee)
        };

        // Re-execute transactions to get BlockExecutionSummary
        // Use saved_no_charge_fee for re-execution without modifying self.no_charge_fee
        let block_exec_summary = self
            .reexecute_preconfirmed_block(&preconfirmed_view, saved_chain_config.as_ref(), saved_no_charge_fee)
            .await
            .context("Re-executing preconfirmed block to get execution summary")?;

        // Extract consumed L1 nonces from transactions
        let consumed_core_contract_nonces: HashSet<u64> = preconfirmed_view
            .borrow_content()
            .executed_transactions()
            .filter_map(|tx| tx.transaction.transaction.as_l1_handler().map(|l1_tx| l1_tx.nonce))
            .collect();

        // Get old_declared_contracts (Cairo 0 legacy classes) - lightweight, no DB queries
        let old_declared_contracts = preconfirmed_view.get_old_declared_contracts();

        // Get deployed contracts set from per-tx state diffs - lightweight, no DB queries
        let deployed_contracts_set = preconfirmed_view.get_deployed_contracts_set();

        // Build set of v2 hashes for SNIP-34 migrated classes
        let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();

        // Convert state_diff with all necessary information
        let state_diff = mp_state_update::StateDiff::from_blockifier(
            block_exec_summary.state_diff,
            &migration_v2_hashes,
            &deployed_contracts_set,
            old_declared_contracts,
        );

        let _db_result = Self::close_preconfirmed_block_with_state_diff(
            self.backend.clone(),
            block_number,
            consumed_core_contract_nonces,
            &block_exec_summary.bouncer_weights,
            state_diff,
        )
        .await
        .context("Closing preconfirmed block on startup")?;

        // Update runtime exec config with current configs after re-execution is complete
        // This ensures that if we restart again before starting the next block, we have the current configs
        // Note: Use self.no_charge_fee (current value) not saved_no_charge_fee (saved value)
        self.save_current_runtime_exec_config()
            .context("Updating runtime execution config after restart re-execution")?;

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

                // Check if pre-confirmed block exists (it shouldn't at this point)
                // Create new preconfirmed block
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

                // Record batch execution stats metrics
                self.metrics.record_execution_stats(&batch_execution_result.stats);

                // Accumulate stats for the log event at block close
                state.accumulated_stats = state.accumulated_stats.clone() + batch_execution_result.stats.clone();

                state.append_batch(batch_execution_result).await?;

                self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
            }
            ExecutorMessage::EndBlock(block_exec_summary) => {
                tracing::debug!("Received ExecutorMessage::EndBlock");
                self.close_block(block_exec_summary).await?;
            }
            ExecutorMessage::EndFinalBlock(block_exec_summary) => {
                tracing::debug!("Received ExecutorMessage::EndFinalBlock (shutdown)");
                match block_exec_summary {
                    Some(summary) => {
                        self.close_block(summary).await?;
                    }
                    None => {
                        tracing::debug!("EndFinalBlock(None) received - executor completed without block");
                    }
                }
            }
        }

        Ok(())
    }

    /// Close and save a block using the execution summary.
    /// Used for both normal block closing (EndBlock) and shutdown (EndFinalBlock).
    async fn close_block(&mut self, block_exec_summary: Box<BlockExecutionSummary>) -> anyhow::Result<()> {
        let current_state = self.current_state.take().context("No current state")?;
        let TaskState::Executing(state) = current_state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
        };

        tracing::debug!("Close and save block block_n={}", state.block_number);
        let start_time = Instant::now();

        // Get preconfirmed block view for transaction count and old_declared_contracts
        let preconfirmed_view = self.backend.block_view_on_preconfirmed().context("No current pre-confirmed block")?;
        let n_txs = preconfirmed_view.num_executed_transactions();
        let event_count = preconfirmed_view
            .borrow_content()
            .executed_transactions()
            .map(|tx| tx.transaction.receipt.events().len() as u64)
            .sum::<u64>();
        let old_declared_contracts = preconfirmed_view.get_old_declared_contracts();

        // Build set of v2 hashes for SNIP-34 migrated classes.
        // These are classes that were USED (not declared) in this block and need their
        // compiled_class_hash updated from Poseidon (v1) to BLAKE (v2).
        let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();

        // Convert state_diff with all necessary information:
        // - migration_v2_hashes: to separate declared vs migrated classes
        // - deployed_contracts: to differentiate deployed vs replaced contracts (computed during batch execution)
        // - old_declared_contracts: Cairo 0 legacy class declarations
        let state_diff = mp_state_update::StateDiff::from_blockifier(
            block_exec_summary.state_diff,
            &migration_v2_hashes,
            &state.deployed_contracts,
            old_declared_contracts,
        );

        // Capture state diff counts before moving state_diff
        let declared_classes_count = state_diff.declared_classes.len();
        let deployed_contracts_count = state_diff.deployed_contracts.len();
        let storage_diffs_count = state_diff.storage_diffs.len();
        let nonce_updates_count = state_diff.nonces.len();
        let state_diff_len = state_diff.len();
        let consumed_l1_nonces_count = state.consumed_core_contract_nonces.len();

        // Capture bouncer weights before moving
        let bouncer_l1_gas = block_exec_summary.bouncer_weights.l1_gas;
        let bouncer_sierra_gas = block_exec_summary.bouncer_weights.sierra_gas.0;
        let bouncer_n_events = block_exec_summary.bouncer_weights.n_events;
        let bouncer_message_segment_length = block_exec_summary.bouncer_weights.message_segment_length;
        let bouncer_state_diff_size = block_exec_summary.bouncer_weights.state_diff_size;

        // Record state diff data gauges before moving state_diff
        self.metrics.block_declared_classes_count.record(declared_classes_count as u64, &[]);
        self.metrics.block_deployed_contracts_count.record(deployed_contracts_count as u64, &[]);
        self.metrics.block_storage_diffs_count.record(storage_diffs_count as u64, &[]);
        self.metrics.block_nonce_updates_count.record(nonce_updates_count as u64, &[]);
        self.metrics.block_state_diff_length.record(state_diff_len as u64, &[]);
        self.metrics.block_event_count.record(event_count, &[]);

        // Record bouncer weights gauges
        self.metrics.block_bouncer_l1_gas.record(bouncer_l1_gas as u64, &[]);
        self.metrics.block_bouncer_sierra_gas.record(bouncer_sierra_gas, &[]);
        self.metrics.block_bouncer_n_events.record(bouncer_n_events as u64, &[]);
        self.metrics.block_bouncer_message_segment_length.record(bouncer_message_segment_length as u64, &[]);
        self.metrics.block_bouncer_state_diff_size.record(bouncer_state_diff_size as u64, &[]);

        // Record consumed L1 nonces count
        self.metrics.block_consumed_l1_nonces_count.record(consumed_l1_nonces_count as u64, &[]);

        let close_preconfirmed_start = Instant::now();
        let db_result = Self::close_preconfirmed_block_with_state_diff(
            self.backend.clone(),
            state.block_number,
            state.consumed_core_contract_nonces,
            &block_exec_summary.bouncer_weights,
            state_diff,
        )
        .await
        .context("Closing block")?;
        let close_preconfirmed_duration = close_preconfirmed_start.elapsed();
        self.metrics.close_preconfirmed_duration.record(close_preconfirmed_duration.as_secs_f64(), &[]);
        self.metrics.close_preconfirmed_last.record(close_preconfirmed_duration.as_secs_f64(), &[]);

        let time_to_close = start_time.elapsed();
        let block_production_time = state.block_start_time.elapsed();

        // Emit structured log event for Loki querying (close_block_complete)
        // All timing values converted to milliseconds for human-readability
        let timings = &db_result.timings;
        let exec_stats = &state.accumulated_stats;
        tracing::info!(
            target: "close_block",
            block_number = state.block_number,
            tx_count = n_txs,
            event_count = event_count,
            // High-level timing
            close_block_total_ms = time_to_close.as_secs_f64() * 1000.0,
            block_close_ms = time_to_close.as_secs_f64() * 1000.0,
            close_preconfirmed_ms = close_preconfirmed_duration.as_secs_f64() * 1000.0,
            block_production_ms = block_production_time.as_secs_f64() * 1000.0,
            // Execution stats
            batches_executed = exec_stats.n_batches,
            txs_added_to_block = exec_stats.n_added_to_block,
            txs_executed = exec_stats.n_executed,
            txs_reverted = exec_stats.n_reverted,
            txs_rejected = exec_stats.n_rejected,
            classes_declared = exec_stats.declared_classes,
            l2_gas_consumed = exec_stats.l2_gas_consumed,
            // State diff counts
            state_diff_len = state_diff_len,
            declared_classes = declared_classes_count,
            deployed_contracts = deployed_contracts_count,
            storage_diffs = storage_diffs_count,
            nonce_updates = nonce_updates_count,
            consumed_l1_nonces = consumed_l1_nonces_count,
            // Bouncer weights
            bouncer_l1_gas = bouncer_l1_gas,
            bouncer_sierra_gas = bouncer_sierra_gas,
            bouncer_n_events = bouncer_n_events,
            bouncer_message_segment_length = bouncer_message_segment_length,
            bouncer_state_diff_size = bouncer_state_diff_size,
            // DB timing breakdown (all in ms)
            get_full_block_ms = timings.get_full_block_with_classes.as_secs_f64() * 1000.0,
            commitments_ms = timings.block_commitments_compute.as_secs_f64() * 1000.0,
            merklization_ms = timings.merklization.as_secs_f64() * 1000.0,
            contract_trie_ms = timings.contract_trie_root.as_secs_f64() * 1000.0,
            class_trie_ms = timings.class_trie_root.as_secs_f64() * 1000.0,
            contract_storage_trie_commit_ms = timings.contract_storage_trie_commit.as_secs_f64() * 1000.0,
            contract_trie_commit_ms = timings.contract_trie_commit.as_secs_f64() * 1000.0,
            class_trie_commit_ms = timings.class_trie_commit.as_secs_f64() * 1000.0,
            block_hash_ms = timings.block_hash_compute.as_secs_f64() * 1000.0,
            db_write_ms = timings.db_write_block_parts.as_secs_f64() * 1000.0,
            "close_block_complete"
        );

        tracing::info!("⛏️  Closed block #{} with {n_txs} transactions - {time_to_close:?}", state.block_number);

        // Record timing metrics
        self.metrics.close_block_total_duration.record(time_to_close.as_secs_f64(), &[]);
        self.metrics.close_block_total_last.record(time_to_close.as_secs_f64(), &[]);

        // Record metrics
        self.metrics.block_counter.add(1, &[]);
        self.metrics.block_gauge.record(state.block_number, &[]);
        self.metrics.transaction_counter.add(n_txs as u64, &[]);
        self.metrics.block_production_time.record(block_production_time.as_secs_f64(), &[]);
        self.metrics.block_production_time_last.record(block_production_time.as_secs_f64(), &[]);
        self.metrics.block_close_time.record(time_to_close.as_secs_f64(), &[]);
        self.metrics.block_close_time_last.record(time_to_close.as_secs_f64(), &[]);

        self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(state.block_number) });
        self.send_state_notification(BlockProductionStateNotification::ClosedBlock);

        Ok(())
    }

    pub(crate) async fn setup_initial_state(&mut self) -> Result<(), anyhow::Error> {
        self.backend.chain_config().precheck_block_production()?;

        self.close_preconfirmed_block_if_exists().await.context("Cannot close preconfirmed block on startup")?;

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
            self.metrics.clone(),
        )
        .context("Starting executor thread")?;

        // Batcher task is handled in a separate tokio task.
        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        // Clone ctx to check for cancellation in the main loop
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

        // Track shutdown state: both batcher and executor must complete before shutdown finishes.
        // Both tasks only complete during shutdown scenarios (cancellation, error, or panic).
        let mut batcher_completed = false;
        let mut end_final_block_received = false; // Track if EndFinalBlock has been processed (executor completed with block)
        let mut executor_stopped = false; // Track if executor.stop has been received (oneshot - can only poll once)
        let mut batcher_error: Option<anyhow::Error> = None; // Store batcher error to return after graceful shutdown

        // Main loop: handles normal operation and graceful shutdown
        loop {
            tokio::select! {
                // Path 1: Batcher task completed (cancellation, error, or channel closure)
                res = &mut batcher_task, if !batcher_completed => {
                    batcher_completed = true;
                    match res {
                        Ok(()) => tracing::debug!("Batcher task completed normally"),
                        Err(e) => {
                            let error = e.context("In batcher task");
                            tracing::warn!("Batcher task errored: {error:?}");
                            batcher_error = Some(error);
                            if self.backend.has_preconfirmed_block() {
                                tracing::warn!("Batcher errored with preconfirmed block, attempting graceful shutdown");
                            }
                        }
                    }
                }

                // Path 2: Executor replies (EndBlock for normal operation, EndFinalBlock for shutdown)
                Some(reply) = executor.replies.recv() => {
                    let is_end_final_block = matches!(reply, ExecutorMessage::EndFinalBlock(_));
                    self.process_reply(reply).await.context("Processing reply from executor thread")?;
                    // Mark executor as completed only after processing EndFinalBlock
                    if is_end_final_block {
                        end_final_block_received = true;
                        tracing::debug!("EndFinalBlock processed, executor completed");
                    }
                }

                // Path 3: Executor thread stopped (normal completion or panic)
                // This fires when executor exits. EndFinalBlock should have been emitted by executor
                // (executor always sends EndFinalBlock during shutdown - Some(summary) if block exists, None if no block).
                // Guard: oneshot channel can only be polled once - polling after completion causes panic.
                res = executor.stop.recv(), if !executor_stopped => {
                    executor_stopped = true;
                    res.context("In executor thread")?;
                }
            }

            // Exit conditions (checked after each select iteration):
            // Shutdown is complete when batcher completed AND EndFinalBlock was processed.
            // Executor always sends EndFinalBlock during shutdown (Some(summary) if block exists, None if no block).
            if batcher_completed && end_final_block_received {
                tracing::debug!("Shutdown complete: batcher completed, EndFinalBlock processed");
                return batcher_error
                    .map(|e| {
                        tracing::warn!("Shutdown completed but batcher had error: {e:?}");
                        Err(e)
                    })
                    .unwrap_or(Ok(()));
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
