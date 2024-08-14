// TODO: Move this into its own crate.

use blockifier::blockifier::transaction_executor::{TransactionExecutor, VisitedSegmentsMapping};
use blockifier::bouncer::{Bouncer, BouncerWeights, BuiltinCount};
use blockifier::state::cached_state::CommitmentStateDiff;
use blockifier::state::state_api::StateReader;
use blockifier::transaction::errors::TransactionExecutionError;
use blockifier::transaction::transaction_execution::Transaction;
use dc_db::db_block_id::DbBlockId;
use dc_db::{DeoxysBackend, DeoxysStorageError};
use dc_exec::{BlockifierStateAdapter, ExecutionContext};
use dp_block::{BlockId, BlockTag, DeoxysPendingBlock};
use dp_class::ConvertedClass;
use dp_convert::ToFelt;
use dp_receipt::from_blockifier_execution_info;
use dp_state_update::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StorageEntry,
};
use dp_transactions::TransactionWithHash;
use dp_utils::graceful_shutdown;
use starknet_types_core::felt::Felt;
use std::mem;
use std::sync::Arc;

use crate::close_block::close_block;
use crate::header::make_pending_header;
use crate::{clone_account_tx, L1DataProvider, Mempool, MempoolTransaction};

/// We always take transactions in batches from the mempool
const TX_BATCH_SIZE: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Storage error: {0:#}")]
    StorageError(#[from] DeoxysStorageError),
    #[error("Execution error: {0:#}")]
    Execution(#[from] TransactionExecutionError),
    #[error(transparent)]
    ExecutionContext(#[from] dc_exec::Error),
    #[error("No genesis block in storage")]
    NoGenesis,
}

fn csd_to_state_diff(
    backend: &DeoxysBackend,
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
    backend: &DeoxysBackend,
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
pub struct BlockProductionTask {
    backend: Arc<DeoxysBackend>,
    mempool: Arc<Mempool>,
    block: DeoxysPendingBlock,
    declared_classes: Vec<ConvertedClass>,
    executor: TransactionExecutor<BlockifierStateAdapter>,
    l1_data_provider: Arc<dyn L1DataProvider>,
    current_pending_tick: usize,
}

impl BlockProductionTask {
    pub fn new(
        backend: Arc<DeoxysBackend>,
        mempool: Arc<Mempool>,
        l1_data_provider: Arc<dyn L1DataProvider>,
    ) -> Result<Self, Error> {
        let parent_block_hash = backend.get_block_hash(&BlockId::Tag(BlockTag::Latest))?.ok_or(Error::NoGenesis)?;
        let pending_block = DeoxysPendingBlock::new_empty(make_pending_header(
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
            ExecutionContext::new(Arc::clone(&backend), &pending_block.info.clone().into())?.tx_executor();

        let bouncer_config = backend.chain_config().bouncer_config.clone();
        executor.bouncer = Bouncer::new(bouncer_config);

        Ok(Self {
            backend,
            mempool,
            executor,
            current_pending_tick: 0,
            block: pending_block,
            declared_classes: vec![],
            l1_data_provider,
        })
    }

    fn continue_block(&mut self, bouncer_cap: BouncerWeights) -> Result<StateDiff, Error> {
        self.executor.bouncer.bouncer_config.block_max_capacity = bouncer_cap;

        let mut txs_to_process = Vec::with_capacity(TX_BATCH_SIZE);
        self.mempool.take_txs_chunk(&mut txs_to_process, TX_BATCH_SIZE);

        let blockifier_txs: Vec<_> =
            txs_to_process.iter().map(|tx| Transaction::AccountTransaction(clone_account_tx(&tx.tx))).collect();

        // Execute the transactions.
        let all_results = self.executor.execute_txs(&blockifier_txs);

        // Split the `txs_to_process` vec into two iterators.
        let mut to_process_iter = txs_to_process.into_iter();
        // This iterator will consume the first part of `to_process_iter`.
        let consumed_txs_to_process = to_process_iter.by_ref().take(all_results.len());

        let on_top_of = self.executor.block_state.as_ref().unwrap().state.on_top_of_block_id;
        let executed_txs: Vec<_> = consumed_txs_to_process.collect();
        let (state_diff, _visited_segments, _weights) =
            finalize_execution_state(&executed_txs, &mut self.executor, &self.backend, &on_top_of)?;

        let n_executed_txs = executed_txs.len();

        for (exec_result, mempool_tx) in Iterator::zip(all_results.into_iter(), executed_txs) {
            match exec_result {
                Ok(execution_info) => {
                    // Note: reverted txs also appear as Ok here.
                    log::debug!("Successful execution of transaction {:?}", mempool_tx.tx_hash());

                    if let Some(class) = mempool_tx.converted_class {
                        self.declared_classes.push(class);
                    }

                    self.block.inner.receipts.push(from_blockifier_execution_info(
                        &execution_info,
                        &Transaction::AccountTransaction(clone_account_tx(&mempool_tx.tx)),
                    ));
                    let converted_tx = TransactionWithHash::from(mempool_tx.tx);
                    self.block.info.tx_hashes.push(converted_tx.hash);
                    self.block.inner.transactions.push(converted_tx.transaction);
                }
                Err(err) => {
                    // TODO: revert handling
                    log::error!("Unsuccessful execution of transaction {:?}: {err:#}", mempool_tx.tx_hash());
                }
            }
        }

        log::debug!(
            "Finished tick with {} new transactions, now at {}",
            n_executed_txs,
            self.block.inner.transactions.len()
        );

        // This contains the rest of `to_process_iter`.
        let rest_txs_to_process: Vec<_> = to_process_iter.collect();

        // Add back the unexecuted transactions to the mempool.
        self.mempool.re_add_txs(rest_txs_to_process);

        Ok(state_diff)
    }

    /// Each "tick" of the block time updates the pending block but only with the appropriate fraction of the total bouncer capacity.
    fn update_pending_block_tick(&mut self) -> Result<(), Error> {
        let current_pending_tick = self.current_pending_tick;
        self.current_pending_tick += 1;

        let n_pending_ticks_per_block = self.backend.chain_config().n_pending_ticks_per_block();

        if current_pending_tick == 0 || current_pending_tick >= n_pending_ticks_per_block {
            // first tick is ignored.
            // out of range ticks are also ignored.
            return Ok(());
        }
        log::debug!("begin pending tick {}/{}", current_pending_tick, n_pending_ticks_per_block);

        // Reduced bouncer capacity for the current pending tick

        let config_bouncer = self.executor.bouncer.bouncer_config.block_max_capacity;
        let frac = n_pending_ticks_per_block / current_pending_tick; // div by zero: current_pending_tick has been checked for 0 above

        log::debug!("frac for this tick: {:.2}", 1f64 / frac as f64);
        let bouncer_cap = BouncerWeights {
            builtin_count: BuiltinCount {
                add_mod: config_bouncer.builtin_count.add_mod / frac,
                bitwise: config_bouncer.builtin_count.bitwise / frac,
                ecdsa: config_bouncer.builtin_count.ecdsa / frac,
                ec_op: config_bouncer.builtin_count.ec_op / frac,
                keccak: config_bouncer.builtin_count.keccak / frac,
                mul_mod: config_bouncer.builtin_count.mul_mod / frac,
                pedersen: config_bouncer.builtin_count.pedersen / frac,
                poseidon: config_bouncer.builtin_count.poseidon / frac,
                range_check: config_bouncer.builtin_count.range_check / frac,
                range_check96: config_bouncer.builtin_count.range_check96 / frac,
            },
            gas: config_bouncer.gas / frac,
            message_segment_length: config_bouncer.message_segment_length / frac,
            n_events: config_bouncer.n_events / frac,
            n_steps: config_bouncer.n_steps / frac,
            state_diff_size: config_bouncer.state_diff_size / frac,
        };

        let state_diff = self.continue_block(bouncer_cap)?;

        // Store pending block
        self.backend.store_block(self.block.clone().into(), state_diff, self.declared_classes.clone())?;

        Ok(())
    }

    fn produce_block_tick(&mut self) -> Result<(), Error> {
        let block_n = self.block_n();
        log::debug!("closing block #{}", block_n);

        // Complete the block with full bouncer capacity.
        let new_state_diff = self.continue_block(self.executor.bouncer.bouncer_config.block_max_capacity)?;

        // Convert the pending block to a closed block and save to db.

        let parent_block_hash = Felt::ZERO; // temp parent block hash
        let new_empty_block = DeoxysPendingBlock::new_empty(make_pending_header(
            parent_block_hash,
            self.backend.chain_config(),
            self.l1_data_provider.as_ref(),
        ));

        let block_to_close = mem::replace(&mut self.block, new_empty_block);
        let declared_classes = mem::take(&mut self.declared_classes);

        // This is compute heavy as it does the commitments and trie computations.
        let chain_id = self.backend.chain_config().chain_id.clone().to_felt();
        let closed_block = close_block(&self.backend, block_to_close, &new_state_diff, chain_id, block_n);
        self.block.info.header.parent_block_hash = closed_block.info.block_hash; // fix temp parent block hash for new pending :)

        self.backend.store_block(closed_block.into(), new_state_diff, declared_classes)?;

        // Prepare for next block.
        self.executor =
            ExecutionContext::new(Arc::clone(&self.backend), &self.block.info.clone().into())?.tx_executor();
        self.current_pending_tick = 0;

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

        log::info!("⛏️  Starting block production on top of block {}", self.block_n());

        loop {
            tokio::select! {
                _ = interval_block_time.tick() => {
                    if let Err(err) = self.produce_block_tick() {
                        log::error!("Block production task has errored: {err:#}");
                    }
                },
                _ = interval_pending_block_update.tick() => {
                    if let Err(err) = self.update_pending_block_tick() {
                        log::error!("Pending block update task has errored: {err:#}");
                    }
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
