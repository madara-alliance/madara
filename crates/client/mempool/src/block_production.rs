//! This should probably be moved into another crate.

use blockifier::blockifier::transaction_executor::{
    TransactionExecutor, TransactionExecutorError, VisitedSegmentsMapping,
};
use blockifier::bouncer::{Bouncer, BouncerWeights, BuiltinCount};
use blockifier::execution::contract_class::ClassInfo;
use blockifier::state::cached_state::{CommitmentStateDiff, StateMaps};
use blockifier::state::state_api::{StateReader, UpdatableState};
use blockifier::transaction::account_transaction::AccountTransaction;
use blockifier::transaction::transaction_execution::Transaction;
use dc_db::db_block_id::DbBlockId;
use dc_db::DeoxysBackend;
use dc_exec::{BlockifierStateAdapter, ExecutionContext};
use dp_block::header::PendingHeader;
use dp_block::{DeoxysBlockInner, DeoxysMaybePendingBlock, DeoxysPendingBlock, DeoxysPendingBlockInfo};
use dp_class::ConvertedClass;
use dp_convert::ToFelt;
use dp_transactions::TransactionWithHash;
use dp_utils::graceful_shutdown;
use itertools::Itertools;
use starknet_api::core::CompiledClassHash;
use starknet_core::types::{
    ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem, NonceUpdate, ReplacedClassItem, StateDiff,
    StateUpdate, StorageEntry,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::close_block::close_block;
use crate::{clone_account_tx, Error, Mempool, MempoolTransaction};

impl PendingState {

    fn finalized_and_store(&self, backend: &DeoxysBackend) {
        let block = close_block(backend, self.make_pending_block().as_nonpending().unwrap(), self.state_diff, chain_id, block_number);
        backend.store_block(
            DeoxysMaybePendingBlock {
                info: dp_block::DeoxysMaybePendingBlockInfo::NotPending(block.info),
                inner: DeoxysBlockInner {
                    transactions: self.txs.iter().map(|tx| tx.transaction),
                    receipts: self.receipts.iter().cloned(),
                },
            },
            self.state_diff,
            self.declared_classes.iter().cloned(),
        )
    }
}

pub struct BlockProductionTask {
    backend: Arc<DeoxysBackend>,
    mempool: Arc<Mempool>,
    block: DeoxysPendingBlock,
    declared_classes: Vec<ConvertedClass>,
    executor: TransactionExecutor<BlockifierStateAdapter>,
    current_pending_tick: usize,
}

const TX_CHUNK_SIZE: usize = 128;

fn csd_to_state_diff(
    backend: &DeoxysBackend,
    on_top_of: &DbBlockId,
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
        let old_class_hash = backend.get_contract_class_hash_at(on_top_of, &contract_address.to_felt())?;
        if old_class_hash.is_some() {
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

pub type TransactionExecutorResult<T> = Result<T, TransactionExecutorError>;
pub const BLOCK_STATE_ACCESS_ERR: &str = "Error: The block state should be `Some`.";
fn get_visited_segments<S: StateReader>(
    tx_executor: &mut TransactionExecutor<S>,
) -> TransactionExecutorResult<VisitedSegmentsMapping> {
    // tx_executor.block_state
    // .as_ref()
    // .expect(BLOCK_STATE_ACCESS_ERR).

    let visited_segments = tx_executor
        .block_state
        .as_ref()
        .expect(BLOCK_STATE_ACCESS_ERR)
        .visited_pcs
        .iter()
        .map(|(class_hash, class_visited_pcs)| -> TransactionExecutorResult<_> {
            let contract_class = tx_executor
                .block_state
                .as_ref()
                .expect(BLOCK_STATE_ACCESS_ERR)
                .get_compiled_contract_class(*class_hash)?;
            Ok((*class_hash, contract_class.get_visited_segments(class_visited_pcs)?))
        })
        .collect::<TransactionExecutorResult<_>>()?;

    Ok(visited_segments)
}

fn finalize_execution_state<S: StateReader>(
    executed_txs: &[MempoolTransaction],
    tx_executor: &mut TransactionExecutor<S>,
    backend: &DeoxysBackend,
    on_top_of: &DbBlockId,
) -> TransactionExecutorResult<(StateDiff, VisitedSegmentsMapping, BouncerWeights)> {
    let csd = tx_executor.block_state.as_mut().expect(BLOCK_STATE_ACCESS_ERR).to_state_diff()?;
    let state_update = csd_to_state_diff(backend, on_top_of, &csd.into())?;

    let visited_segments = get_visited_segments(tx_executor)?;

    log::debug!("Final block weights: {:?}.", tx_executor.bouncer.get_accumulated_weights());
    Ok((
        state_update,
        visited_segments,
        *tx_executor.bouncer.get_accumulated_weights(),
    ))
}

impl BlockProductionTask {
    pub fn new(backend: Arc<DeoxysBackend>, mempool: Arc<Mempool>) -> Result<Self, Error> {
        let pending_block = mempool.get_or_create_pending_block()?;
        let mut executor = ExecutionContext::new(Arc::clone(&backend), &pending_block.info)?.tx_executor();

        let bouncer_config = backend.chain_config().bouncer_config.clone();
        executor.bouncer = Bouncer::new(bouncer_config);

        Ok(Self { backend, mempool, executor, current_pending_tick: 0, block: block.as_nonpending().unwrap(), declared_classes })
    }

    fn store_pending(&self, backend: &DeoxysBackend,
        state_diff: StateDiff,
        _visited_segments: VisitedSegmentsMapping,
    ) {
        backend.store_block(
            self.pending_block,
            self.state_diff,
            self.declared_classes.iter().cloned(),
        )
    }

    fn store_tx(&self, tx: &DeclareTransaction) {
        tx
    }

    fn update_pending_block(&mut self, finish_block: bool) -> Result<(), Error> {
        let config_bouncer = &self.backend.chain_config().bouncer_config.block_max_capacity;

        if finish_block {
            // Full bouncer capacity
            self.executor.bouncer.bouncer_config.block_max_capacity = config_bouncer.clone();
        } else {
            let current_pending_tick = self.current_pending_tick;
            self.current_pending_tick += 1;

            let n_pending_ticks_per_block = self.backend.chain_config().block_time.as_millis()
                / self.backend.chain_config().pending_block_update_time.as_millis();
            let n_pending_ticks_per_block = n_pending_ticks_per_block as usize;

            if current_pending_tick == 0 || current_pending_tick >= n_pending_ticks_per_block {
                // first tick is ignored.
                // out of range ticks are also ignored.
                return Ok(());
            }

            // Reduced bouncer capacity for the current pending tick

            // (current_val / n_ticks_per_block) * current_tick_in_block
            let frac = n_pending_ticks_per_block * current_pending_tick;
            self.executor.bouncer.bouncer_config.block_max_capacity = BouncerWeights {
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
        }

        let mut txs_to_process = Vec::with_capacity(TX_CHUNK_SIZE);
        self.mempool.take_txs_chunk(&mut txs_to_process, TX_CHUNK_SIZE);

        let blockifier_txs: Vec<_> =
            txs_to_process.iter().map(|tx| Transaction::AccountTransaction(clone_account_tx(&tx.tx))).collect();

        // Execute the transactions.
        let all_results = self.executor.execute_txs(&blockifier_txs);

        // Split the `txs_to_process` vec into two iterators.
        let mut to_process_iter = txs_to_process.into_iter();
        // This iterator will consume the first part of `to_process_iter`.
        let consumed_txs_to_process = to_process_iter.by_ref().take(all_results.len());

        let excuted_txs: Vec<_> = consumed_txs_to_process.collect();
        let (state_diff, visited_segments, _weights) = finalize_execution_state(&executed_txs, &mut self.executor, &self.backend, self.executor.block_state.unwrap().state.on_top_of_block_id)?;

        for (exec_result, mempool_tx) in Iterator::zip(all_results.into_iter(), &excuted_txs) {
            match exec_result {
                Ok(execution_info) => {
                    // Note: reverted txs also appear as Ok here.
                    log::debug!("Successful execution of transaction {:?}", mempool_tx.tx_hash());

                    if let AccountTransaction::Declare(tx) = mempool_tx {
                        self.store_class(tx)
                    }

                    self.block.inner.transactions.push(mempool_tx.tx.into());
                    self.block.inner.receipts.push(execution_info.transaction_receipt.into());

                    Some(execution_info.transaction_receipt)
                }
                Err(err) => {
                    log::error!("Unsuccessful execution of transaction {:?}: {err:#}", mempool_tx.tx_hash());
                    None
                }
            }
        }

        // This contains the rest of `to_process_iter`.
        let rest_txs_to_process: Vec<_> = to_process_iter.collect();

        // Add back the unexecuted transactions to the mempool.
        self.mempool.readd_txs(rest_txs_to_process);

        self.store_pending(backend, state_diff, visited_segments)?;

        Ok(())
    }

    fn produce_block(&mut self) -> Result<(), Error> {
        // Complete the block with full bouncer capacity.
        let finish_block = true;
        self.update_pending_block(finish_block)?;

        // Convert the pending block to a closed block and save to db.

        // Prepare for next block.
        let mut pending_block = self.mempool.get_or_create_pending_block()?;
        let bouncer_config = self.backend.chain_config().bouncer_config.clone();
        self.executor = ExecutionContext::new(Arc::clone(&self.backend), &pending_block.info)?.tx_executor();
        self.current_pending_tick = 0;

        Ok(())
    }

    pub async fn block_production_task(&mut self) -> Result<(), anyhow::Error> {
        let start = tokio::time::Instant::now();

        let mut interval_block_time = tokio::time::interval_at(start, self.backend.chain_config().block_time);
        interval_block_time.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut interval_pending_block_update =
            tokio::time::interval_at(start, self.backend.chain_config().pending_block_update_time);
        interval_pending_block_update.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = interval_block_time.tick() => {
                    if let Err(err) = self.produce_block() {
                        log::error!("Block production task has errored: {err:#}");
                    }
                },
                _ = interval_pending_block_update.tick() => {
                    if let Err(err) = self.update_pending_block(false) {
                        log::error!("Pending block update task has errored: {err:#}");
                    }
                },
                _ = graceful_shutdown() => break,
            }
        }

        Ok(())
    }
}
