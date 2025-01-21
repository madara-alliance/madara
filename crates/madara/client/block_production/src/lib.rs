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

use crate::close_block::close_block;
use crate::metrics::BlockProductionMetrics;
use blockifier::blockifier::transaction_executor::{TransactionExecutor, BLOCK_STATE_ACCESS_ERR};
use blockifier::bouncer::BouncerWeights;
use blockifier::transaction::errors::TransactionExecutionError;
use finalize_execution_state::StateDiffToStateMapError;
use mc_block_import::{BlockImportError, BlockImporter};
use mc_db::db_block_id::DbBlockId;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::{BlockifierStateAdapter, ExecutionContext};
use mc_mempool::header::make_pending_header;
use mc_mempool::{L1DataProvider, MempoolProvider};
use mp_block::{BlockId, BlockTag, MadaraPendingBlock, VisitedSegments};
use mp_class::compile::ClassCompilationError;
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{ContractStorageDiffItem, NonceUpdate, StateDiff, StorageEntry};
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
mod re_add_finalized_to_blockifier;

#[derive(Default, Clone)]
struct ContinueBlockStats {
    /// Number of batches executed before reaching the bouncer capacity.
    pub n_batches: usize,
    /// Number of transactions included into the block
    pub n_added_to_block: usize,
    pub n_re_added_to_mempool: usize,
    pub n_reverted: usize,
    /// Rejected are txs that were unsucessful and but that were not revertible.
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
    #[error("Import error: {0:#}")]
    Import(#[from] mc_block_import::BlockImportError),
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

    /// Tracks which segments of Cairo program code were accessed during transaction execution,
    /// organized by class hash. This information is used as input for SNOS (Starknet OS)
    /// when generating proofs of execution.
    visited_segments: VisitedSegments,

    /// The current state of resource consumption tracked by the bouncer
    bouncer_weights: BouncerWeights,

    /// Statistics about transaction processing during this continuation
    stats: ContinueBlockStats,

    /// Indicates whether the block reached its resource limits during this continuation.
    /// When true, no more transactions can be added to the current block.
    block_now_full: bool,
}

/// The block production task consumes transactions from the mempool in batches.
/// This is to allow optimistic concurrency. However, the block may get full during batch execution,
/// and we need to re-add the transactions back into the mempool.
///
/// To understand block production in madara, you should probably start with the [`mp_chain_config::ChainConfig`]
/// documentation.
pub struct BlockProductionTask<Mempool: MempoolProvider> {
    importer: Arc<BlockImporter>,
    backend: Arc<MadaraBackend>,
    mempool: Arc<Mempool>,
    block: MadaraPendingBlock,
    declared_classes: Vec<ConvertedClass>,
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

    /// Continue the pending block state by re-adding all of its transactions back into the mempool.
    /// This function will always clear the pending block in db, even if the transactions could not be added to the mempool.
    pub fn re_add_pending_block_txs_to_mempool(
        backend: &MadaraBackend,
        mempool: &Mempool,
    ) -> Result<(), Cow<'static, str>> {
        let Some(current_pending_block) =
            backend.get_block(&DbBlockId::Pending).map_err(|err| format!("Getting pending block: {err:#}"))?
        else {
            // No pending block
            return Ok(());
        };
        backend.clear_pending_block().map_err(|err| format!("Clearing pending block: {err:#}"))?;

        let n_txs = re_add_finalized_to_blockifier::re_add_txs_to_mempool(current_pending_block, mempool, backend)
            .map_err(|err| format!("Re-adding transactions to mempool: {err:#}"))?;

        if n_txs > 0 {
            tracing::info!("🔁 Re-added {n_txs} transactions from the pending block back into the mempool");
        }
        Ok(())
    }

    pub fn new(
        backend: Arc<MadaraBackend>,
        importer: Arc<BlockImporter>,
        mempool: Arc<Mempool>,
        metrics: Arc<BlockProductionMetrics>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> Result<Self, Error> {
        if let Err(err) = Self::re_add_pending_block_txs_to_mempool(&backend, &mempool) {
            // This error should not stop block production from working. If it happens, that's too bad. We drop the pending state and start from
            // a fresh one.
            tracing::error!("Failed to continue the pending block state: {err:#}");
        }

        let parent_block_hash = backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))?
            .unwrap_or(/* genesis block's parent hash */ Felt::ZERO);

        let pending_block = MadaraPendingBlock::new_empty(make_pending_header(
            parent_block_hash,
            backend.chain_config(),
            l1_data_provider.as_ref(),
        ));

        let executor = ExecutionContext::new_at_block_start(Arc::clone(&backend), &pending_block.info.clone().into())?
            .tx_executor();

        Ok(Self {
            importer,
            backend,
            mempool,
            executor,
            current_pending_tick: 0,
            block: pending_block,
            declared_classes: Default::default(),
            l1_data_provider,
            metrics,
        })
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

                txs_to_process_blockifier.extend(txs_to_process.iter().skip(cur_len).map(|tx| tx.clone_tx()));
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
                    Ok(execution_info) => {
                        // Reverted transactions appear here as Ok too.
                        tracing::debug!("Successful execution of transaction {:#x}", mempool_tx.tx_hash().to_felt());

                        stats.n_added_to_block += 1;
                        if execution_info.is_reverted() {
                            stats.n_reverted += 1;
                        }

                        if let Some(class) = mem::take(&mut mempool_tx.converted_class) {
                            self.declared_classes.push(class);
                        }

                        self.block
                            .inner
                            .receipts
                            .push(from_blockifier_execution_info(&execution_info, &mempool_tx.clone_tx()));
                        let converted_tx = TransactionWithHash::from(mempool_tx.clone_tx());
                        self.block.info.tx_hashes.push(converted_tx.hash);
                        self.block.inner.transactions.push(converted_tx.transaction);
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

        let (state_diff, visited_segments, bouncer_weights) =
            finalize_execution_state::finalize_execution_state(&mut self.executor, &self.backend, &on_top_of)?;

        // Add back the unexecuted transactions to the mempool.
        stats.n_re_added_to_mempool = txs_to_process.len();
        self.mempool
            .txs_re_add(txs_to_process, executed_txs)
            .map_err(|err| Error::Unexpected(format!("Mempool error: {err:#}").into()))?;

        tracing::debug!(
            "Finished tick with {} new transactions, now at {} - re-adding {} txs to mempool",
            stats.n_added_to_block,
            self.block.inner.transactions.len(),
            stats.n_re_added_to_mempool
        );

        Ok(ContinueBlockResult { state_diff, visited_segments, bouncer_weights, stats, block_now_full })
    }

    /// Closes the current block and prepares for the next one
    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    async fn close_and_prepare_next_block(
        &mut self,
        state_diff: StateDiff,
        visited_segments: VisitedSegments,
        start_time: Instant,
    ) -> Result<(), Error> {
        let block_n = self.block_n();
        // Convert the pending block to a closed block and save to db
        let parent_block_hash = Felt::ZERO; // temp parent block hash
        let new_empty_block = MadaraPendingBlock::new_empty(make_pending_header(
            parent_block_hash,
            self.backend.chain_config(),
            self.l1_data_provider.as_ref(),
        ));

        let block_to_close = mem::replace(&mut self.block, new_empty_block);
        let declared_classes = mem::take(&mut self.declared_classes);

        let n_txs = block_to_close.inner.transactions.len();

        // Close and import the block
        let import_result = close_block(
            &self.importer,
            block_to_close,
            &state_diff,
            self.backend.chain_config().chain_id.clone(),
            block_n,
            declared_classes,
            visited_segments,
        )
        .await?;

        // Removes nonces in the mempool nonce cache which have been included
        // into the current block.
        for NonceUpdate { contract_address, .. } in state_diff.nonces.iter() {
            self.mempool.tx_mark_included(contract_address);
        }

        // Flush changes to disk
        self.backend.flush().map_err(|err| BlockImportError::Internal(format!("DB flushing error: {err:#}").into()))?;

        // Update parent hash for new pending block
        self.block.info.header.parent_block_hash = import_result.block_hash;

        // Prepare executor for next block
        self.executor =
            ExecutionContext::new_at_block_start(Arc::clone(&self.backend), &self.block.info.clone().into())?
                .tx_executor();
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

        let ContinueBlockResult {
            state_diff: mut new_state_diff,
            visited_segments,
            bouncer_weights,
            stats,
            block_now_full,
        } = self.continue_block(self.backend.chain_config().bouncer_config.block_max_capacity)?;

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
            self.close_and_prepare_next_block(new_state_diff, visited_segments, start_time).await?;
            return Ok(true);
        }

        // Store pending block
        // todo, prefer using the block import pipeline?
        self.backend.store_block(
            self.block.clone().into(),
            new_state_diff,
            self.declared_classes.clone(),
            None,
            Some(visited_segments),
            Some(bouncer_weights),
        )?;
        // do not forget to flush :)
        self.backend.flush().map_err(|err| BlockImportError::Internal(format!("DB flushing error: {err:#}").into()))?;

        Ok(false)
    }

    /// This creates a block, continuing the current pending block state up to the full bouncer limit.
    #[tracing::instrument(skip(self), fields(module = "BlockProductionTask"))]
    pub(crate) async fn on_block_time(&mut self) -> Result<(), Error> {
        let block_n = self.block_n();
        tracing::debug!("closing block #{}", block_n);

        // Complete the block with full bouncer capacity
        let start_time = Instant::now();
        let ContinueBlockResult {
            state_diff: mut new_state_diff,
            visited_segments,
            bouncer_weights: _weights,
            stats: _stats,
            block_now_full: _block_now_full,
        } = self.continue_block(self.backend.chain_config().bouncer_config.block_max_capacity)?;

        self.update_block_hash_registry(&mut new_state_diff, block_n)?;

        self.close_and_prepare_next_block(new_state_diff, visited_segments, start_time).await
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
    use std::{collections::HashMap, sync::Arc};

    use blockifier::{compiled_class_hash, nonce, state::cached_state::StateMaps, storage_key};
    use mc_db::MadaraBackend;
    use mp_chain_config::ChainConfig;
    use mp_convert::ToFelt;
    use mp_state_update::{
        ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, StateDiff, StorageEntry,
    };
    use starknet_api::{
        class_hash, contract_address,
        core::{ClassHash, ContractAddress, PatriciaKey},
        felt, patricia_key,
    };
    use starknet_types_core::felt::Felt;

    use crate::finalize_execution_state::state_map_to_state_diff;

    #[test]
    fn test_state_map_to_state_diff() {
        let backend = MadaraBackend::open_for_testing(Arc::new(ChainConfig::madara_test()));

        let mut nonces = HashMap::new();
        nonces.insert(contract_address!(1u32), nonce!(1));
        nonces.insert(contract_address!(2u32), nonce!(2));
        nonces.insert(contract_address!(3u32), nonce!(3));

        let mut class_hashes = HashMap::new();
        class_hashes.insert(contract_address!(1u32), class_hash!("0xc1a551"));
        class_hashes.insert(contract_address!(2u32), class_hash!("0xc1a552"));
        class_hashes.insert(contract_address!(3u32), class_hash!("0xc1a553"));

        let mut storage = HashMap::new();
        storage.insert((contract_address!(1u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(1u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(1u32), storage_key!(3u32)), felt!(3u32));

        storage.insert((contract_address!(2u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(2u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(2u32), storage_key!(3u32)), felt!(3u32));

        storage.insert((contract_address!(3u32), storage_key!(1u32)), felt!(1u32));
        storage.insert((contract_address!(3u32), storage_key!(2u32)), felt!(2u32));
        storage.insert((contract_address!(3u32), storage_key!(3u32)), felt!(3u32));

        let mut compiled_class_hashes = HashMap::new();
        // "0xc1a553" is marked as deprecated by not having a compiled
        // class hashe
        compiled_class_hashes.insert(class_hash!("0xc1a551"), compiled_class_hash!(0x1));
        compiled_class_hashes.insert(class_hash!("0xc1a552"), compiled_class_hash!(0x2));

        let mut declared_contracts = HashMap::new();
        declared_contracts.insert(class_hash!("0xc1a551"), true);
        declared_contracts.insert(class_hash!("0xc1a552"), true);
        declared_contracts.insert(class_hash!("0xc1a553"), true);

        let state_map = StateMaps { nonces, class_hashes, storage, compiled_class_hashes, declared_contracts };

        let storage_diffs = vec![
            ContractStorageDiffItem {
                address: felt!(1u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!(2u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
            ContractStorageDiffItem {
                address: felt!(3u32),
                storage_entries: vec![
                    StorageEntry { key: felt!(1u32), value: Felt::ONE },
                    StorageEntry { key: felt!(2u32), value: Felt::TWO },
                    StorageEntry { key: felt!(3u32), value: Felt::THREE },
                ],
            },
        ];

        let deprecated_declared_classes = vec![class_hash!("0xc1a553").to_felt()];

        let declared_classes = vec![
            DeclaredClassItem {
                class_hash: class_hash!("0xc1a551").to_felt(),
                compiled_class_hash: compiled_class_hash!(0x1).to_felt(),
            },
            DeclaredClassItem {
                class_hash: class_hash!("0xc1a552").to_felt(),
                compiled_class_hash: compiled_class_hash!(0x2).to_felt(),
            },
        ];

        let nonces = vec![
            NonceUpdate { contract_address: felt!(1u32), nonce: felt!(1u32) },
            NonceUpdate { contract_address: felt!(2u32), nonce: felt!(2u32) },
            NonceUpdate { contract_address: felt!(3u32), nonce: felt!(3u32) },
        ];

        let deployed_contracts = vec![
            DeployedContractItem { address: felt!(1u32), class_hash: class_hash!("0xc1a551").to_felt() },
            DeployedContractItem { address: felt!(2u32), class_hash: class_hash!("0xc1a552").to_felt() },
            DeployedContractItem { address: felt!(3u32), class_hash: class_hash!("0xc1a553").to_felt() },
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
}
