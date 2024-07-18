//! This should probably be moved into another crate.

use blockifier::bouncer::{Bouncer, BouncerWeights, BuiltinCount};
use blockifier::transaction::transaction_execution::Transaction;
use dc_db::DeoxysBackend;
use dc_exec::ExecutionContext;
use dp_utils::graceful_shutdown;
use std::sync::Arc;

use crate::{clone_account_tx, Error, Mempool};

pub struct BlockProductionTask {
    backend: Arc<DeoxysBackend>,
    mempool: Arc<Mempool>,
    bouncer: Bouncer,

    current_pending_tick: usize,
}

const TX_CHUNK_SIZE: usize = 128;

impl BlockProductionTask {
    pub fn new(backend: Arc<DeoxysBackend>, mempool: Arc<Mempool>) -> Self {
        let bouncer_config = backend.chain_config().bouncer_config.clone();
        Self { backend, mempool, bouncer: Bouncer::new(bouncer_config), current_pending_tick: 0 }
    }
    fn update_pending_block(&mut self, finish_block: bool) -> Result<(), Error> {
        let config_bouncer = &self.backend.chain_config().bouncer_config.block_max_capacity;

        if finish_block {
            // Full bouncer capacity
            self.bouncer.bouncer_config.block_max_capacity = config_bouncer.clone();
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
            self.bouncer.bouncer_config.block_max_capacity = BouncerWeights {
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

        let mut pending_block = self.mempool.get_or_create_pending_block()?;
        let mut executor = ExecutionContext::new(&self.backend, &pending_block.info)?.tx_executor();

        let mut txs_to_process = Vec::with_capacity(TX_CHUNK_SIZE);
        self.mempool.take_txs_chunk(&mut txs_to_process, TX_CHUNK_SIZE);

        let blockifier_txs: Vec<_> =
            txs_to_process.iter().map(|tx| Transaction::AccountTransaction(clone_account_tx(&tx.tx))).collect();

        // Execute the transactions.
        let all_results = executor.execute_txs(&blockifier_txs);

        // Split the `txs_to_process` vec into two iterators.
        let mut to_process_iter = txs_to_process.into_iter();
        // This iterator will consume the first part of `to_process_iter`.
        let consumed_txs_to_process = to_process_iter.by_ref().take(all_results.len());

        for (exec_result, mempool_tx) in Iterator::zip(all_results.into_iter(), consumed_txs_to_process) {
            match exec_result {
                Ok(execution_info) => {
                    log::debug!("Successful execution of {:?}", mempool_tx.tx_hash());
                }
                Err(err) => {
                    log::debug!("Unsuccessful execution of {:?}: {err:#}", mempool_tx.tx_hash());
                }
            }

            
            // Append to the pending block.
            // pending_block
        }

        // This contains the rest of `to_process_iter`.
        let rest_txs_to_process: Vec<_> = to_process_iter.collect();

        // Add back the unexecuted transactions to the mempool.
        self.mempool.readd_txs(rest_txs_to_process);

        Ok(())
    }

    fn produce_block(&mut self) -> Result<(), Error> {
        // Complete the block with full bouncer capacity.
        let finish_block = true;
        self.update_pending_block(finish_block)?;

        // Reset bouncer and current tick.
        let bouncer_config = self.backend.chain_config().bouncer_config.clone();
        self.bouncer = Bouncer::new(bouncer_config);
        self.current_pending_tick = 0;

        // Convert the pending block to a closed block and save to db.


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
