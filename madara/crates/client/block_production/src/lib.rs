//! Block production service.
//!
//! # Testing
//!
//! Testing is done in a few places:
//! - devnet has a few tests for declare transactions and basic transfers as of now. This is proobably
//!   the simplest place where we could add more tests about block-time, mempool saving to db and such.
//! - e2e tests test a few transactions, with the rpc/gateway in scope too.
//! - js-tests in the CI, not that much in depth
//! - higher level, block production is more hreavily tested (currently outside of the CI) by running the
//!   bootstrapper and the kakarot test suite. This is the only place where L1-L2 messaging is really tested
//!   as of now. We should make better tests around this area.
//!
//! There are no tests in this crate because they would require a proper genesis block. Devnet provides that,
//! so that's where block-production integration tests are the simplest to add.
//! L1-L2 testing is a bit harder to setup, but we should definitely make the testing more comprehensive here.

use crate::close_block::close_and_save_block;
use crate::metrics::BlockProductionMetrics;
use anyhow::Context;
use blockifier::blockifier::transaction_executor::{TransactionExecutor, BLOCK_STATE_ACCESS_ERR};
use blockifier::bouncer::BouncerWeights;
use blockifier::transaction::errors::TransactionExecutionError;
use finalize_execution_state::StateDiffToStateMapError;
use mc_db::db_block_id::DbBlockId;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::{BlockifierStateAdapter, ExecutionContext};
use mc_mempool::header::make_pending_header;
use mc_mempool::{L1DataProvider, MempoolProvider};
use mp_block::header::PendingHeader;
use mp_block::{BlockId, BlockTag, MadaraPendingBlockInfo, PendingFullBlock, TransactionWithReceipt};
use mp_class::compile::ClassCompilationError;
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::{from_blockifier_execution_info, EventWithTransactionHash};
use mp_state_update::{ContractStorageDiffItem, DeclaredClassItem, NonceUpdate, StateDiff, StorageEntry};
use mp_transactions::TransactionWithHash;
use mp_utils::service::ServiceContext;
use opentelemetry::KeyValue;
use starknet_types_core::felt::Felt;
use std::borrow::Cow;
use std::collections::VecDeque;
use std::mem;
use std::sync::Arc;
use std::time::Instant;

mod close_block;
mod finalize_execution_state;
pub mod metrics;

#[derive(Default, Clone, Debug)]
struct ContinueBlockStats {
    /// Number of batches executed before reaching the bouncer capacity.
    pub n_batches: usize,
    /// Number of transactions included into the block.
    pub n_added_to_block: usize,
    /// Transactions that were popped from the mempool but not executed, and so they are re-added back into the mempool.
    pub n_re_added_to_mempool: usize,
    /// Rejected transactions are failing transactions that are included in the block.
    pub n_reverted: usize,
    /// Rejected are txs are failing transactions that are not revertible. They are thus not included in the block
    pub n_rejected: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] MadaraStorageError),
    #[error("Execution error: {0:#}")]
    Execution(#[from] TransactionExecutionError),
    #[error(transparent)]
    ExecutionContext(#[from] mc_exec::Error),
    #[error("Unexpected error: {0:#}")]
    Unexpected(Cow<'static, str>),
    #[error("Class compilation error when continuing the pending block: {0:#}")]
    PendingClassCompilationError(#[from] ClassCompilationError),
    #[error("State diff error when continuing the pending block: {0:#}")]
    PendingStateDiff(#[from] StateDiffToStateMapError),
}

/// Result of a block continuation operation, containing the updated state and execution statistics.
/// This is returned by [`BlockProductionTask::continue_block`] when processing a batch of transactions.
struct ContinueBlockResult {
    /// The accumulated state changes from executing transactions in this continuation
    state_diff: StateDiff,

    /// The current state of resource consumption tracked by the bouncer
    #[allow(unused)]
    bouncer_weights: BouncerWeights,

    /// Statistics about transaction processing during this continuation
    stats: ContinueBlockStats,

    /// Indicates whether the block reached its resource limits during this continuation.
    /// When true, no more transactions can be added to the current block.
    block_now_full: bool,
}

#[derive(Debug, Clone)]
struct PendingBlockState {
    pub header: PendingHeader,
    pub transactions: Vec<TransactionWithReceipt>,
    pub events: Vec<EventWithTransactionHash>,
    pub declared_classes: Vec<ConvertedClass>,
}

impl PendingBlockState {
    pub fn new(header: PendingHeader) -> Self {
        Self { header, transactions: vec![], events: vec![], declared_classes: vec![] }
    }

    pub fn into_full_block_with_classes(self, state_diff: StateDiff) -> (PendingFullBlock, Vec<ConvertedClass>) {
        (
            PendingFullBlock { header: self.header, state_diff, transactions: self.transactions, events: self.events },
            self.declared_classes,
        )
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

/// The block production task consumes transactions from the mempool in batches.
///
/// This is to allow optimistic concurrency. However, the block may get full during batch execution,
/// and we need to re-add the transactions back into the mempool.
///
/// To understand block production in madara, you should probably start with the [`mp_chain_config::ChainConfig`]
/// documentation.
pub struct BlockProductionTask<Mempool: MempoolProvider> {
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    pending_block: PendingBlockState,
    pub(crate) executor: TransactionExecutor<BlockifierStateAdapter>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    current_pending_tick: usize,
    metrics: Arc<BlockProductionMetrics>,
}

impl<Mempool: MempoolProvider> BlockProductionTask<Mempool> {
    #[cfg(any(test, feature = "testing"))]
    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    pub fn set_current_pending_tick(&mut self, n: usize) {
        self.current_pending_tick = n;
    }

    /// Closes the last pending block store in db (if any).
    ///
    /// This avoids re-executing transaction by re-adding them to the [Mempool],
    /// as was done before.
    pub async fn close_pending_block(backend: &MadaraBackend, metrics: &BlockProductionMetrics) -> anyhow::Result<()> {
        let start_time = Instant::now();

        // We cannot use `backend.get_block` to check for the existence of the
        // pending block as it will ALWAYS return a pending block, even if there
        // is none in db (it uses the Default::default in that case).
        if !backend.has_pending_block().context("Error checking if pending block exists")? {
            return Ok(());
        }

        let (block, declared_classes) = get_pending_block_from_db(backend)?;

        // NOTE: we disabled the Write Ahead Log when clearing the pending block
        // so this will be done atomically at the same time as we close the next
        // block, after we manually flush the db.
        backend.clear_pending_block().context("Error clearing pending block")?;

        let block_n = backend.get_latest_block_n().context("Getting latest block n")?.map(|n| n + 1).unwrap_or(0);
        let n_txs = block.transactions.len();

        // Close and import the pending block
        close_and_save_block(backend, block, declared_classes, block_n)
            .await
            .context("Failed to close pending block")?;

        // Flush changes to disk, pending block removal and adding the next
        // block happens atomically
        backend.flush().context("DB flushing error")?;

        let end_time = start_time.elapsed();
        tracing::info!("⛏️  Closed block #{} with {} transactions - {:?}", block_n, n_txs, end_time);

        // Record metrics
        let attributes = [
            KeyValue::new("transactions_added", n_txs.to_string()),
            KeyValue::new("closing_time", end_time.as_secs_f32().to_string()),
        ];

        metrics.block_counter.add(1, &[]);
        metrics.block_gauge.record(block_n, &attributes);
        metrics.transaction_counter.add(n_txs as u64, &[]);

        Ok(())
    }

    pub async fn new(
        backend: Arc<MadaraBackend>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> Result<Self, Error> {
        if let Err(err) = Self::close_pending_block(&backend, &metrics).await {
            // This error should not stop block production from working. If it happens, that's too bad. We drop the pending state and start from
            // a fresh one.
            tracing::error!("Failed to continue the pending block state: {err:#}");
        }

        let parent_block_hash = backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))?
            .unwrap_or(/* genesis block's parent hash */ Felt::ZERO);

        let pending_block = PendingBlockState::new(make_pending_header(
            parent_block_hash,
            backend.chain_config(),
            l1_data_provider.as_ref(),
        ));

        let executor = ExecutionContext::new_at_block_start(
            Arc::clone(&backend),
            &mp_block::MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                header: pending_block.header.clone(),
                tx_hashes: vec![],
            }),
        )?
        .executor_for_block_production();

        Ok(Self { backend, mempool, executor, current_pending_tick: 0, pending_block, l1_data_provider, metrics })
    }

    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    fn continue_block(&mut self, bouncer_cap: BouncerWeights) -> Result<ContinueBlockResult, Error> {
        let mut stats = ContinueBlockStats::default();
        let mut block_now_full = false;

        self.executor.bouncer.bouncer_config.block_max_capacity = bouncer_cap;
        let batch_size = self.backend.chain_config().execution_batch_size;

        let mut txs_to_process = VecDeque::with_capacity(batch_size);
        let mut txs_to_process_blockifier = Vec::with_capacity(batch_size);
        // This does not need to be outside the loop, but that saves an allocation
        let mut executed_txs = Vec::with_capacity(batch_size);

        // Cloning transactions: That's a lot of cloning, but we're kind of forced to do that because blockifier takes
        // a `&[Transaction]` slice. In addition, declare transactions have their class behind an Arc.
        loop {
            // Take transactions from mempool.
            let to_take = batch_size.saturating_sub(txs_to_process.len());
            let cur_len = txs_to_process.len();
            if to_take > 0 {
                self.mempool.txs_take_chunk(/* extend */ &mut txs_to_process, batch_size);

                txs_to_process_blockifier.extend(txs_to_process.iter().skip(cur_len).map(|tx| tx.tx.clone()));
            }

            if txs_to_process.is_empty() {
                // Not enough transactions in mempool to make a new batch.
                break;
            }

            stats.n_batches += 1;

            // Execute the transactions.
            let all_results = self.executor.execute_txs(&txs_to_process_blockifier);
            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            block_now_full = all_results.len() < txs_to_process_blockifier.len();

            txs_to_process_blockifier.drain(..all_results.len()); // remove the used txs

            for exec_result in all_results {
                let mut mempool_tx =
                    txs_to_process.pop_front().ok_or_else(|| Error::Unexpected("Vector length mismatch".into()))?;

                // Remove tx from mempool
                self.backend.remove_mempool_transaction(&mempool_tx.tx_hash().to_felt())?;

                match exec_result {
                    Ok((execution_info, _)) => {
                        // Reverted transactions appear here as Ok too.
                        tracing::debug!("Successful execution of transaction {:#x}", mempool_tx.tx_hash().to_felt());

                        stats.n_added_to_block += 1;
                        if execution_info.is_reverted() {
                            stats.n_reverted += 1;
                        }

                        let receipt = from_blockifier_execution_info(&execution_info, &mempool_tx.tx);
                        let converted_tx = TransactionWithHash::from(mempool_tx.tx.clone());

                        if let Some(class) = mempool_tx.converted_class.take() {
                            self.pending_block.declared_classes.push(class);
                        }
                        self.pending_block.events.extend(
                            receipt
                                .events()
                                .iter()
                                .cloned()
                                .map(|event| EventWithTransactionHash { event, transaction_hash: converted_tx.hash }),
                        );
                        self.pending_block
                            .transactions
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
                        stats.n_rejected += 1;
                    }
                }

                executed_txs.push(mempool_tx)
            }

            if block_now_full {
                break;
            }
        }

        let on_top_of = self.executor.block_state.as_ref().expect(BLOCK_STATE_ACCESS_ERR).state.on_top_of_block_id;

        let (state_diff, bouncer_weights) =
            finalize_execution_state::finalize_execution_state(&mut self.executor, &self.backend, &on_top_of)?;

        // Add back the unexecuted transactions to the mempool.
        stats.n_re_added_to_mempool = txs_to_process.len();
        self.mempool
            .txs_re_add(txs_to_process, executed_txs)
            .map_err(|err| Error::Unexpected(format!("Mempool error: {err:#}").into()))?;

        tracing::debug!(
            "Finished tick with {} new transactions, now at {} - re-adding {} txs to mempool",
            stats.n_added_to_block,
            self.pending_block.transactions.len(),
            stats.n_re_added_to_mempool
        );

        Ok(ContinueBlockResult { state_diff, bouncer_weights, stats, block_now_full })
    }

    /// Closes the current block and prepares for the next one
    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    async fn close_and_prepare_next_block(&mut self, state_diff: StateDiff, start_time: Instant) -> Result<(), Error> {
        let block_n = self.block_n();
        // Convert the pending block to a closed block and save to db
        let parent_block_hash = Felt::ZERO; // temp parent block hash
        let new_empty_block = PendingBlockState::new(make_pending_header(
            parent_block_hash,
            self.backend.chain_config(),
            self.l1_data_provider.as_ref(),
        ));

        let block_to_close = mem::replace(&mut self.pending_block, new_empty_block);
        let (full_pending_block, classes) = block_to_close.into_full_block_with_classes(state_diff.clone());

        let n_txs = full_pending_block.transactions.len();

        // Close and import the block
        let block_hash = close_and_save_block(&self.backend, full_pending_block, classes, block_n)
            .await
            .map_err(|err| Error::Unexpected(format!("Error closing block: {err:#}").into()))?;

        // Removes nonces in the mempool nonce cache which have been included
        // into the current block.
        for NonceUpdate { contract_address, .. } in state_diff.nonces.iter() {
            self.mempool.tx_mark_included(contract_address);
        }

        // Flush changes to disk
        self.backend.flush().map_err(|err| Error::Unexpected(format!("DB flushing error: {err:#}").into()))?;

        // Update parent hash for new pending block
        self.pending_block.header.parent_block_hash = block_hash;

        // Prepare executor for next block
        self.executor = ExecutionContext::new_at_block_start(
            Arc::clone(&self.backend),
            &mp_block::MadaraMaybePendingBlockInfo::Pending(MadaraPendingBlockInfo {
                header: self.pending_block.header.clone(),
                tx_hashes: vec![],
            }),
        )?
        .executor_for_block_production();
        self.current_pending_tick = 0;

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

        Ok(())
    }

    /// Updates the state diff to store a block hash at the special address 0x1, which serves as
    /// Starknet's block hash registry.
    ///
    /// # Purpose
    /// Address 0x1 in Starknet is a special contract address that maintains a mapping of block numbers
    /// to their corresponding block hashes. This storage is used by the `get_block_hash` system call
    /// and is essential for block hash verification within the Starknet protocol.
    ///
    /// # Storage Structure at Address 0x1
    /// - Keys: Block numbers
    /// - Values: Corresponding block hashes
    /// - Default: 0 for all other block numbers
    ///
    /// # Implementation Details
    /// For each block N ≥ 10, this function stores the hash of block (N-10) at address 0x1
    /// with the block number as the key.
    ///
    /// For more details, see the [official Starknet documentation on special addresses]
    /// (https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1)
    ///
    /// It is also required by SNOS for PIEs creation of the block.
    fn update_block_hash_registry(&self, state_diff: &mut StateDiff, block_n: u64) -> Result<(), Error> {
        if block_n >= 10 {
            let prev_block_number = block_n - 10;
            let prev_block_hash = self
                .backend
                .get_block_hash(&BlockId::Number(prev_block_number))
                .map_err(|err| {
                    Error::Unexpected(
                        format!("Error fetching block hash for block {prev_block_number}: {err:#}").into(),
                    )
                })?
                .ok_or_else(|| {
                    Error::Unexpected(format!("No block hash found for block number {prev_block_number}").into())
                })?;

            state_diff.storage_diffs.push(ContractStorageDiffItem {
                address: Felt::ONE, // Address 0x1
                storage_entries: vec![StorageEntry { key: Felt::from(prev_block_number), value: prev_block_hash }],
            });
        }
        Ok(())
    }

    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    pub async fn on_pending_time_tick(&mut self) -> Result<bool, Error> {
        let current_pending_tick = self.current_pending_tick;
        if current_pending_tick == 0 {
            return Ok(false);
        }

        let start_time = Instant::now();

        tracing::debug!("{:?}", self.backend.chain_config().bouncer_config.block_max_capacity);

        let ContinueBlockResult { state_diff: mut new_state_diff, bouncer_weights: _, stats, block_now_full } =
            self.continue_block(self.backend.chain_config().bouncer_config.block_max_capacity)?;

        tracing::debug!("{:?} {:?}", stats, block_now_full);

        if stats.n_added_to_block > 0 {
            tracing::info!(
                "🧮 Executed and added {} transaction(s) to the pending block at height {} - {:?}",
                stats.n_added_to_block,
                self.block_n(),
                start_time.elapsed(),
            );
        }

        // Check if block is full
        if block_now_full {
            let block_n = self.block_n();
            self.update_block_hash_registry(&mut new_state_diff, block_n)?;

            tracing::info!("Resource limits reached, closing block early");
            self.close_and_prepare_next_block(new_state_diff, start_time).await?;
            return Ok(true);
        }

        // Store pending block
        // self.backend.store_pending_block(block)
        let (block, classes) = self.pending_block.clone().into_full_block_with_classes(new_state_diff);
        self.backend.store_pending_block_with_classes(block, &classes)?;
        // do not forget to flush :)
        self.backend.flush().map_err(|err| Error::Unexpected(format!("DB flushing error: {err:#}").into()))?;

        Ok(false)
    }

    /// This creates a block, continuing the current pending block state up to the full bouncer limit.
    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    pub(crate) async fn on_block_time(&mut self) -> Result<(), Error> {
        let block_n = self.block_n();
        tracing::debug!("Closing block #{}", block_n);

        // Complete the block with full bouncer capacity
        let start_time = Instant::now();
        let ContinueBlockResult { state_diff: mut new_state_diff, .. } =
            self.continue_block(self.backend.chain_config().bouncer_config.block_max_capacity)?;

        self.update_block_hash_registry(&mut new_state_diff, block_n)?;

        self.close_and_prepare_next_block(new_state_diff, start_time).await
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn block_production_task(mut self, mut ctx: ServiceContext) -> Result<(), anyhow::Error> {
        let start = tokio::time::Instant::now();

        let mut interval_block_time = tokio::time::interval_at(start, self.backend.chain_config().block_time);
        interval_block_time.reset(); // do not fire the first tick immediately
        interval_block_time.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut interval_pending_block_update =
            tokio::time::interval_at(start, self.backend.chain_config().pending_block_update_time);
        interval_pending_block_update.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        self.backend.chain_config().precheck_block_production()?; // check chain config for invalid config

        tracing::info!("⛏️  Starting block production at block #{}", self.block_n());

        loop {
            tokio::select! {
                instant = interval_block_time.tick() => {
                    if let Err(err) = self.on_block_time().await {
                        tracing::error!("Block production task has errored: {err:#}");
                        // Clear pending block. The reason we do this is because
                        // if the error happened because the closed block is
                        // invalid or has not been saved properly, we want to
                        // avoid redoing the same error in the next block. So we
                        // drop all the transactions in the pending block just
                        // in case. If the problem happened after the block was
                        // closed and saved to the db, this will do nothing.
                        if let Err(err) = self.backend.clear_pending_block() {
                            tracing::error!("Error while clearing the pending block in recovery of block production error: {err:#}");
                        }
                    }
                    // ensure the pending block tick and block time match up
                    interval_pending_block_update.reset_at(instant + interval_pending_block_update.period());
                },
                instant = interval_pending_block_update.tick() => {
                    let n_pending_ticks_per_block = self.backend.chain_config().n_pending_ticks_per_block();

                    if self.current_pending_tick == 0 || self.current_pending_tick >= n_pending_ticks_per_block {
                        // First tick is ignored. Out of range ticks are also
                        // ignored.
                        self.current_pending_tick += 1;
                        continue
                    }

                    match self.on_pending_time_tick().await {
                        Ok(block_closed) => {
                            if block_closed {
                                interval_pending_block_update.reset_at(instant + interval_pending_block_update.period());
                                interval_block_time.reset_at(instant + interval_block_time.period());
                                self.current_pending_tick = 0;
                            } else {
                                self.current_pending_tick += 1;
                            }
                        }
                        Err(err) => {
                            tracing::error!("Pending block update task has errored: {err:#}");
                        }
                    }
                },
                _ = ctx.cancelled() => break,
            }
        }

        Ok(())
    }

    fn block_n(&self) -> u64 {
        self.executor.block_context.block_info().block_number.0
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        finalize_execution_state::state_map_to_state_diff, get_pending_block_from_db, metrics::BlockProductionMetrics,
        BlockProductionTask, PendingBlockState,
    };
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
    use starknet_api::core::{ClassHash, CompiledClassHash, Nonce};
    use starknet_types_core::felt::Felt;
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use tracing_test::traced_test;

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
        setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>),
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
        let (backend, metrics) = setup;

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
        BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics)
            .await
            .expect("Failed to close pending block");

        // Now we check this was the case.
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

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
        setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>),

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
        let (backend, metrics) = setup;

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
        BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics)
            .await
            .expect("Failed to close pending block");

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
    #[tokio::test]
    async fn block_prod_pending_close_on_startup_no_pending(setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>)) {
        let (backend, metrics) = setup;

        // Simulates starting block production without a pending block in db
        BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics)
            .await
            .expect("Failed to close pending block");

        // Now we check no block was added to the db
        assert_eq!(backend.get_latest_block_n().unwrap(), None);
    }

    /// This test makes sure that if a pending block is already present in db
    /// at startup, then it is closed and stored in db, event if has no visited
    /// segments.
    ///
    /// This will arise if switching from a full node to a sequencer with the
    /// same db.
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_no_visited_segments(
        setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>),
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
        let (backend, metrics) = setup;

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
                // None, // No visited segments!
            )
            .expect("Failed to store pending block");

        // ================================================================== //
        //        PART 3: init block production and seal pending block        //
        // ================================================================== //

        // This should load the pending block from db and close it
        BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics)
            .await
            .expect("Failed to close pending block");

        // Now we check this was the case.
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

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

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing class.
    #[rstest::rstest]
    #[traced_test]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class(
        setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>),
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let (backend, metrics) = setup;

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
        let err =
            BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics).await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    /// This test makes sure that closing the pending block from db will fail if
    /// the pending state diff references a non-existing legacy class.
    #[rstest::rstest]
    #[tokio::test]
    #[traced_test]
    #[allow(clippy::too_many_arguments)]
    async fn block_prod_pending_close_on_startup_fail_missing_class_legacy(
        setup: (Arc<MadaraBackend>, Arc<BlockProductionMetrics>),
        #[with(Felt::ONE)] tx_invoke_v0: TxFixtureInfo,
        #[with(Felt::TWO)] tx_l1_handler: TxFixtureInfo,
        #[with(Felt::THREE)] tx_declare_v0: TxFixtureInfo,
        tx_deploy: TxFixtureInfo,
        tx_deploy_account: TxFixtureInfo,
    ) {
        let (backend, metrics) = setup;

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
        let err =
            BlockProductionTask::<Mempool>::close_pending_block(&backend, &metrics).await.expect_err("Should error");

        assert!(format!("{err:#}").contains("not found"), "{err:#}");
    }

    // This test makes sure that the pending tick updates correctly
    // the pending block without closing if the bouncer limites aren't reached
    #[rstest::rstest]
    #[tokio::test]
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
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: call on pending time tick                 //
        // ================================================================== //

        // The block should still be pending since we haven't
        // reached the block limit and there should be no new
        // finalized blocks
        block_production_task.set_current_pending_tick(1);
        block_production_task.on_pending_time_tick().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert_eq!(pending_block.inner.transactions.len(), 1);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);
    }

    // This test makes sure that the pending tick updates the correct
    // pending block if a new pending block is added to the database
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_updates_correct_block(
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
        tx_invoke_v0: TxFixtureInfo,
        #[from(tx_l1_handler)]
        #[with(Felt::ONE)]
        tx_l1_handler: TxFixtureInfo,
        #[from(tx_declare_v0)]
        #[with(Felt::TWO)]
        tx_declare_v0: TxFixtureInfo,
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
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                PART 3: call on pending time tick once              //
        // ================================================================== //

        block_production_task.set_current_pending_tick(1);
        block_production_task.on_pending_time_tick().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert_eq!(pending_block.inner.transactions.len(), 1);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 4: we prepare the pending block              //
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
        //                 PART 5: storing the pending block                  //
        // ================================================================== //

        // We insert a pending block to check if the block production task
        // keeps a consistent state
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
        //           PART 6: add more transactions to the mempool             //
        // ================================================================== //

        sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //             PART 7: call on pending time tick again                //
        // ================================================================== //

        block_production_task.on_pending_time_tick().await.unwrap();

        let pending_block = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert_eq!(pending_block.inner.transactions.len(), 2);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);
    }

    // This test makes sure that the pending tick closes the block
    // if the bouncer capacity is reached
    #[rstest::rstest]
    #[tokio::test]
    #[traced_test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_pending_block_tick_closes_block(
        #[future]
        #[with(16, Duration::from_secs(60000), Duration::from_millis(3), true)]
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
        sign_and_add_declare_tx(&contracts.0[1], &backend, &tx_validator, Felt::ZERO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: call on pending time tick                 //
        // ================================================================== //

        // The BouncerConfig is set up with amounts (100000) that should limit
        // the block size in a way that the pending tick on this task
        // closes the block
        block_production_task.set_current_pending_tick(1);
        block_production_task.on_pending_time_tick().await.unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
        assert!(!mempool.is_empty());
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
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                      PART 3: call on block time                    //
        // ================================================================== //

        block_production_task.on_block_time().await.unwrap();

        let pending_block = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

    // This test checks that the task fails to close the block
    // if the block it's working on if forcibly change to one
    // that isn't consistent with the previous state
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_on_block_time_fails_inconsistent_state(
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
        tx_invoke_v0: TxFixtureInfo,
        #[from(tx_l1_handler)]
        #[with(Felt::ONE)]
        tx_l1_handler: TxFixtureInfo,
        #[from(tx_declare_v0)]
        #[with(Felt::TWO)]
        tx_declare_v0: TxFixtureInfo,
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
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert_eq!(pending_block.inner.transactions.len(), 0);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: we prepare the pending block              //
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
        //                 PART 4: storing the pending block                  //
        // ================================================================== //

        // We insert a pending block to check if the block production task
        // keeps a consistent state
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
        //                 PART 5: changing the task pending block            //
        // ================================================================== //

        // Here we purposefully change the block being worked on
        let (pending_block, classes) = get_pending_block_from_db(&backend).unwrap();
        block_production_task.pending_block = PendingBlockState {
            header: pending_block.header,
            transactions: pending_block.transactions,
            events: pending_block.events,
            declared_classes: classes,
        };

        // ================================================================== //
        //                      PART 6: call on block time                    //
        // ================================================================== //

        // If the program ran correctly, the pending block should
        // have no transactions on it after the method ran for at
        // least block_time

        block_production_task.on_block_time().await.unwrap();

        assert!(mempool.is_empty());
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);
    }

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

        let block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let task_handle = tokio::spawn(async move {
            block_production_task.block_production_task(mp_utils::service::ServiceContext::new_for_testing()).await
        });

        // We abort after the minimum execution time
        // plus a little bit to guarantee
        tokio::time::sleep(std::time::Duration::from_secs(31)).await;
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

        let block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let result = block_production_task
            .block_production_task(mp_utils::service::ServiceContext::new_for_testing())
            .await
            .expect_err("Should give an error");

        assert_eq!(result.to_string(), "Block time cannot be zero for block production.");
        assert!(!mempool.is_empty());
    }

    // This test tries to overload the tokio tick system between
    // updating pending block and closing it, but it should work properly
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_closes_block_right_after_pending(
        #[future]
        #[with(1, Duration::from_micros(1002), Duration::from_micros(1001), false)]
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
        sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        let mut block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();
        block_production_task.set_current_pending_tick(1);

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                  PART 3: init block production task                //
        // ================================================================== //

        let task_handle = tokio::spawn(async move {
            block_production_task.block_production_task(mp_utils::service::ServiceContext::new_for_testing()).await
        });

        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        task_handle.abort();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        let block_inner = backend
            .get_block(&mp_block::BlockId::Number(1))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;

        assert_eq!(block_inner.transactions.len(), 2);
        assert!(mempool.is_empty());
        assert!(pending_block.inner.transactions.is_empty());
    }

    // This test shuts down the block production task mid execution
    // and creates a new one to check if it correctly set up the new state
    #[rstest::rstest]
    #[tokio::test]
    #[allow(clippy::too_many_arguments)]
    async fn test_block_prod_start_block_production_task_ungracious_shutdown_and_restart(
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
        //             PART 1: we add a transaction to the mempool            //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        assert!(mempool.is_empty());
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::ZERO).await;
        sign_and_add_invoke_tx(&contracts.0[0], &contracts.0[1], &backend, &tx_validator, Felt::ONE).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 2: create block production task                //
        // ================================================================== //

        // I don't really understand why Arc::clone doesn't work with dyn
        // but .clone() works, so I had to make due
        let block_production_task = BlockProductionTask::new(
            Arc::clone(&backend),
            Arc::clone(&mempool),
            Arc::clone(&metrics),
            l1_data_provider.clone(),
        )
        .await
        .unwrap();

        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 0);

        // ================================================================== //
        //                PART 3: init block production task                  //
        // ================================================================== //

        let ctx = mp_utils::service::ServiceContext::new_for_testing();

        let task_handle = tokio::spawn(async move { block_production_task.block_production_task(ctx).await });
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        task_handle.abort();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert_eq!(pending_block.inner.transactions.len(), 2);

        // ================================================================== //
        //           PART 4: we add more transactions to the mempool          //
        // ================================================================== //

        // The transaction itself is meaningless, it's just to check
        // if the task correctly reads it and process it
        sign_and_add_declare_tx(&contracts.0[0], &backend, &tx_validator, Felt::TWO).await;
        assert!(!mempool.is_empty());

        // ================================================================== //
        //                PART 5: create block production task                //
        // ================================================================== //

        // This should seal the previous pending block
        let block_production_task =
            BlockProductionTask::new(Arc::clone(&backend), Arc::clone(&mempool), metrics, l1_data_provider)
                .await
                .unwrap();

        let block_inner = backend
            .get_block(&mp_block::BlockId::Tag(mp_block::BlockTag::Latest))
            .expect("Failed to retrieve latest block from db")
            .expect("Missing latest block")
            .inner;

        assert_eq!(block_inner.transactions.len(), 2);
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 1);

        // ================================================================== //
        //             PART 6: we start block production again                //
        // ================================================================== //

        let task_handle = tokio::spawn(async move {
            block_production_task.block_production_task(mp_utils::service::ServiceContext::new_for_testing()).await
        });

        tokio::time::sleep(std::time::Duration::from_secs(35)).await;
        task_handle.abort();

        let pending_block: mp_block::MadaraMaybePendingBlock = backend.get_block(&DbBlockId::Pending).unwrap().unwrap();

        assert!(mempool.is_empty());
        assert!(pending_block.inner.transactions.is_empty());
        assert_eq!(backend.get_latest_block_n().unwrap().unwrap(), 2);
    }
}
