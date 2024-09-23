// TODO: Move this into its own crate.

use blockifier::blockifier::transaction_executor::{TransactionExecutor, VisitedSegmentsMapping};
use blockifier::bouncer::{Bouncer, BouncerWeights, BuiltinCount};
use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::state::state_api::StateReader;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::transaction_execution::Transaction;
use mc_block_import::{BlockImportError, BlockImporter};
use mc_db::db_block_id::DbBlockId;
use mc_db::{MadaraBackend, MadaraStorageError};
use mc_exec::{BlockifierStateAdapter, ExecutionContext};
use mp_block::{BlockId, BlockTag, MadaraPendingBlock};
use mp_class::ConvertedClass;
use mp_convert::ToFelt;
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use mp_transactions::TransactionWithHash;
use mp_utils::graceful_shutdown;
use starknet_types_core::felt::Felt;
use std::collections::VecDeque;
use std::mem;
use std::sync::Arc;
use std::time::Instant;

use crate::close_block::close_block;
use crate::header::make_pending_header;
use crate::{clone_account_tx, L1DataProvider, MempoolProvider, MempoolTransaction};

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
}

fn csd_to_state_diff(
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
    csd: &CommitmentStateDiff,
) -> Result<StateDiff, Error> {
    let CommitmentStateDiff {
        address_to_class_hash,
        address_to_nonce,
        storage_updates,
        class_hash_to_compiled_class_hash,
    } = csd;

    let (mut deployed_contracts, mut replaced_classes) = (Vec::new(), Vec::new());
    for (contract_address, new_class_hash) in address_to_class_hash {
        let replaced = if let Some(on_top_of) = on_top_of {
            backend.get_contract_class_hash_at(on_top_of, &contract_address.to_felt())?.is_some()
        } else {
            // Executing genesis block: nothing being redefined here
            false
        };
        if replaced {
            replaced_classes.push(ReplacedClassItem {
                contract_address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        } else {
            deployed_contracts.push(DeployedContractItem {
                address: contract_address.to_felt(),
                class_hash: new_class_hash.to_felt(),
            })
        }
    }

    Ok(StateDiff {
        storage_diffs: storage_updates
            .into_iter()
            .map(|(address, storage_entries)| ContractStorageDiffItem {
                address: address.to_felt(),
                storage_entries: storage_entries
                    .into_iter()
                    .map(|(key, value)| StorageEntry { key: key.to_felt(), value: *value })
                    .collect(),
            })
            .collect(),
        deprecated_declared_classes: vec![],
        declared_classes: class_hash_to_compiled_class_hash
            .iter()
            .map(|(class_hash, compiled_class_hash)| DeclaredClassItem {
                class_hash: class_hash.to_felt(),
                compiled_class_hash: compiled_class_hash.to_felt(),
            })
            .collect(),
        nonces: address_to_nonce
            .into_iter()
            .map(|(contract_address, nonce)| NonceUpdate {
                contract_address: contract_address.to_felt(),
                nonce: nonce.to_felt(),
            })
            .collect(),
        deployed_contracts,
        replaced_classes,
    })
}

pub const BLOCK_STATE_ACCESS_ERR: &str = "Error: The block state should be `Some`.";
fn get_visited_segments<S: StateReader>(
    tx_executor: &mut TransactionExecutor<S>,
) -> Result<VisitedSegmentsMapping, Error> {
    let visited_segments = tx_executor
        .block_state
        .as_ref()
        .expect(BLOCK_STATE_ACCESS_ERR)
        .visited_pcs
        .iter()
        .map(|(class_hash, class_visited_pcs)| -> Result<_, Error> {
            let contract_class = tx_executor
                .block_state
                .as_ref()
                .expect(BLOCK_STATE_ACCESS_ERR)
                .get_compiled_contract_class(*class_hash)
                .map_err(TransactionExecutionError::StateError)?;
            Ok((*class_hash, contract_class.get_visited_segments(class_visited_pcs)?))
        })
        .collect::<Result<_, Error>>()?;

    Ok(visited_segments)
}

fn finalize_execution_state<S: StateReader>(
    _executed_txs: &[MempoolTransaction],
    tx_executor: &mut TransactionExecutor<S>,
    backend: &MadaraBackend,
    on_top_of: &Option<DbBlockId>,
) -> Result<(StateDiff, VisitedSegmentsMapping, BouncerWeights), Error> {
    let csd = tx_executor
        .block_state
        .as_mut()
        .expect(BLOCK_STATE_ACCESS_ERR)
        .to_state_diff()
        .map_err(TransactionExecutionError::StateError)?;
    let state_update = csd_to_state_diff(backend, on_top_of, &csd.into())?;

    let visited_segments = get_visited_segments(tx_executor)?;

    Ok((state_update, visited_segments, *tx_executor.bouncer.get_accumulated_weights()))
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
}

impl<Mempool: MempoolProvider> BlockProductionTask<Mempool> {
    #[cfg(any(test, feature = "testing"))]
    pub fn set_current_pending_tick(&mut self, n: usize) {
        self.current_pending_tick = n;
    }

    pub fn new(
        backend: Arc<MadaraBackend>,
        importer: Arc<BlockImporter>,
        mempool: Arc<Mempool>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> Result<Self, Error> {
        let parent_block_hash = backend
            .get_block_hash(&BlockId::Tag(BlockTag::Latest))?
            .unwrap_or(/* genesis block's parent hash */ Felt::ZERO);
        let pending_block = MadaraPendingBlock::new_empty(make_pending_header(
            parent_block_hash,
            backend.chain_config(),
            l1_data_provider.as_ref(),
        ));
        // NB: we cannot continue a previously started pending block yet.
        // let pending_block = backend.get_or_create_pending_block(|| CreatePendingBlockExtraInfo {
        //     l1_gas_price: l1_data_provider.get_gas_prices(),
        //     l1_da_mode: l1_data_provider.get_da_mode(),
        // })?;
        let mut executor =
            ExecutionContext::new_in_block(Arc::clone(&backend), &pending_block.info.clone().into())?.tx_executor();

        let bouncer_config = backend.chain_config().bouncer_config.clone();
        executor.bouncer = Bouncer::new(bouncer_config);

        Ok(Self {
            importer,
            backend,
            mempool,
            executor,
            current_pending_tick: 0,
            block: pending_block,
            declared_classes: vec![],
            l1_data_provider,
        })
    }

    fn continue_block(&mut self, bouncer_cap: BouncerWeights) -> Result<(StateDiff, ContinueBlockStats), Error> {
        let mut stats = ContinueBlockStats::default();

        self.executor.bouncer.bouncer_config.block_max_capacity = bouncer_cap;
        let batch_size = self.backend.chain_config().execution_batch_size;

        let mut txs_to_process = VecDeque::with_capacity(batch_size);
        let mut txs_to_process_blockifier = Vec::with_capacity(batch_size);
        // This does not need to be outside the loop, but that saves an allocation
        let mut executed_txs = Vec::with_capacity(batch_size);

        loop {
            // Take transactions from mempool.
            let to_take = batch_size.saturating_sub(txs_to_process.len());
            if to_take > 0 {
                self.mempool.take_txs_chunk(/* extend */ &mut txs_to_process, batch_size);
            }

            if txs_to_process.is_empty() {
                // Not enough transactions in mempool to make a new batch.
                break;
            }

            stats.n_batches += 1;

            txs_to_process_blockifier
                .extend(txs_to_process.iter().map(|tx| Transaction::AccountTransaction(clone_account_tx(&tx.tx))));

            // Execute the transactions.
            let all_results = self.executor.execute_txs(&txs_to_process_blockifier);
            // When the bouncer cap is reached, blockifier will return fewer results than what we asked for.
            let block_now_full = all_results.len() < txs_to_process_blockifier.len();

            for exec_result in all_results {
                let mut mempool_tx = txs_to_process.pop_front().expect("Vector length mismatch");
                match exec_result {
                    Ok(execution_info) => {
                        // Reverted transactions appear here as Ok too.
                        log::debug!("Successful execution of transaction {:#x}", mempool_tx.tx_hash().to_felt());

                        stats.n_added_to_block += 1;
                        if execution_info.is_reverted() {
                            stats.n_reverted += 1;
                        }

                        if let Some(class) = mem::take(&mut mempool_tx.converted_class) {
                            self.declared_classes.push(class);
                        }

                        self.block.inner.receipts.push(from_blockifier_execution_info(
                            &execution_info,
                            &Transaction::AccountTransaction(clone_account_tx(&mempool_tx.tx)),
                        ));
                        let converted_tx = TransactionWithHash::from(clone_account_tx(&mempool_tx.tx)); // TODO: too many tx clones!
                        self.block.info.tx_hashes.push(converted_tx.hash);
                        self.block.inner.transactions.push(converted_tx.transaction);
                    }
                    Err(err) => {
                        // These are the transactions that have errored but we can't revert them. It can be because of an internal server error, but
                        // errors during the execution of Declare and DeployAccount also appear here as they cannot be reverted.
                        // We reject them.
                        // Note that this is a big DoS vector.
                        log::error!("Rejected transaction {} for unexpected error: {err:#}", mempool_tx.tx_hash());
                        stats.n_rejected += 1;
                    }
                }

                executed_txs.push(mempool_tx)
            }

            if block_now_full {
                break;
            }
        }

        // Add back the unexecuted transactions to the mempool.
        stats.n_re_added_to_mempool = txs_to_process.len();
        self.mempool.re_add_txs(txs_to_process);

        let on_top_of = self
            .executor
            .block_state
            .as_ref()
            .expect("Block state can not be None unless we take ownership of it")
            .state
            .on_top_of_block_id;

        let (state_diff, _visited_segments, _weights) =
            finalize_execution_state(&executed_txs, &mut self.executor, &self.backend, &on_top_of)?;

        log::debug!(
            "Finished tick with {} new transactions, now at {} - re-adding {} txs to mempool",
            stats.n_added_to_block,
            self.block.inner.transactions.len(),
            stats.n_re_added_to_mempool
        );

        Ok((state_diff, stats))
    }

    /// Each "tick" of the block time updates the pending block but only with the appropriate fraction of the total bouncer capacity.
    pub fn on_pending_time_tick(&mut self) -> Result<(), Error> {
        let current_pending_tick = self.current_pending_tick;
        let n_pending_ticks_per_block = self.backend.chain_config().n_pending_ticks_per_block();
        let config_bouncer = self.backend.chain_config().bouncer_config.block_max_capacity;
        if current_pending_tick == 0 {
            return Ok(());
        }

        // Reduced bouncer capacity for the current pending tick

        // reduced_gas = gas * current_pending_tick/n_pending_ticks_per_block
        // - we're dealing with integers here so prefer having the division last
        // - use u128 here because the multiplication would overflow
        // - div by zero: see [`ChainConfig::precheck_block_production`]
        let reduced_cap =
            |v: usize| (v as u128 * current_pending_tick as u128 / n_pending_ticks_per_block as u128) as usize;

        let gas = reduced_cap(config_bouncer.gas);
        let frac = current_pending_tick as f64 / n_pending_ticks_per_block as f64;
        log::debug!("begin pending tick {current_pending_tick}/{n_pending_ticks_per_block}, proportion for this tick: {frac:.2}, gas limit: {gas}/{}", config_bouncer.gas);

        let bouncer_cap = BouncerWeights {
            builtin_count: BuiltinCount {
                add_mod: reduced_cap(config_bouncer.builtin_count.add_mod),
                bitwise: reduced_cap(config_bouncer.builtin_count.bitwise),
                ecdsa: reduced_cap(config_bouncer.builtin_count.ecdsa),
                ec_op: reduced_cap(config_bouncer.builtin_count.ec_op),
                keccak: reduced_cap(config_bouncer.builtin_count.keccak),
                mul_mod: reduced_cap(config_bouncer.builtin_count.mul_mod),
                pedersen: reduced_cap(config_bouncer.builtin_count.pedersen),
                poseidon: reduced_cap(config_bouncer.builtin_count.poseidon),
                range_check: reduced_cap(config_bouncer.builtin_count.range_check),
                range_check96: reduced_cap(config_bouncer.builtin_count.range_check96),
            },
            gas,
            message_segment_length: reduced_cap(config_bouncer.message_segment_length),
            n_events: reduced_cap(config_bouncer.n_events),
            n_steps: reduced_cap(config_bouncer.n_steps),
            state_diff_size: reduced_cap(config_bouncer.state_diff_size),
        };

        let start_time = Instant::now();
        let (state_diff, stats) = self.continue_block(bouncer_cap)?;
        if stats.n_added_to_block > 0 {
            log::info!(
                "🧮 Executed and added {} transaction(s) to the pending block at height {} - {:?}",
                stats.n_added_to_block,
                self.block_n(),
                start_time.elapsed(),
            );
        }

        // Store pending block
        // todo, prefer using the block import pipeline?
        self.backend.store_block(self.block.clone().into(), state_diff, self.declared_classes.clone())?;
        // do not forget to flush :)
        self.backend
            .maybe_flush(true)
            .map_err(|err| BlockImportError::Internal(format!("DB flushing error: {err:#}").into()))?;

        Ok(())
    }

    /// This creates a block, continuing the current pending block state up to the full bouncer limit.
    pub(crate) async fn on_block_time(&mut self) -> Result<(), Error> {
        let block_n = self.block_n();
        log::debug!("closing block #{}", block_n);

        // Complete the block with full bouncer capacity.
        let start_time = Instant::now();
        let (new_state_diff, _n_executed) =
            self.continue_block(self.backend.chain_config().bouncer_config.block_max_capacity)?;

        // Convert the pending block to a closed block and save to db.

        let parent_block_hash = Felt::ZERO; // temp parent block hash
        let new_empty_block = MadaraPendingBlock::new_empty(make_pending_header(
            parent_block_hash,
            self.backend.chain_config(),
            self.l1_data_provider.as_ref(),
        ));

        let block_to_close = mem::replace(&mut self.block, new_empty_block);
        let declared_classes = mem::take(&mut self.declared_classes);

        let n_txs = block_to_close.inner.transactions.len();

        // This is compute heavy as it does the commitments and trie computations.
        let import_result = close_block(
            &self.importer,
            block_to_close,
            &new_state_diff,
            self.backend.chain_config().chain_id.clone(),
            block_n,
            declared_classes,
        )
        .await?;
        self.block.info.header.parent_block_hash = import_result.block_hash; // fix temp parent block hash for new pending :)

        // Prepare for next block.
        self.executor =
            ExecutionContext::new_in_block(Arc::clone(&self.backend), &self.block.info.clone().into())?.tx_executor();
        self.current_pending_tick = 0;

        log::info!("⛏️  Closed block #{} with {} transactions - {:?}", block_n, n_txs, start_time.elapsed());

        Ok(())
    }

    pub async fn block_production_task(&mut self) -> Result<(), anyhow::Error> {
        let start = tokio::time::Instant::now();

        let mut interval_block_time = tokio::time::interval_at(start, self.backend.chain_config().block_time);
        interval_block_time.reset(); // do not fire the first tick immediately
        interval_block_time.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut interval_pending_block_update =
            tokio::time::interval_at(start, self.backend.chain_config().pending_block_update_time);
        interval_pending_block_update.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        self.backend.chain_config().precheck_block_production()?; // check chain config for invalid config

        log::info!("⛏️  Starting block production at block #{}", self.block_n());

        loop {
            tokio::select! {
                instant = interval_block_time.tick() => {
                    if let Err(err) = self.on_block_time().await {
                        log::error!("Block production task has errored: {err:#}");
                    }
                    // ensure the pending block tick and block time match up
                    interval_pending_block_update.reset_at(instant + interval_pending_block_update.period());
                },
                _ = interval_pending_block_update.tick() => {
                    let n_pending_ticks_per_block = self.backend.chain_config().n_pending_ticks_per_block();

                    if self.current_pending_tick == 0 || self.current_pending_tick >= n_pending_ticks_per_block {
                        // first tick is ignored.
                        // out of range ticks are also ignored.
                        self.current_pending_tick += 1;
                        continue
                    }

                    if let Err(err) = self.on_pending_time_tick() {
                        log::error!("Pending block update task has errored: {err:#}");
                    }
                    self.current_pending_tick += 1;
                },
                _ = graceful_shutdown() => break,
            }
        }

        Ok(())
    }

    fn block_n(&self) -> u64 {
        self.executor.block_context.block_info().block_number.0
    }
}
