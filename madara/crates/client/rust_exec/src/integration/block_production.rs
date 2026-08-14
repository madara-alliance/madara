//! Block production integration for rust-exec.
//!
//! This module is responsible for executing rust-exec supported transactions
//! while producing Blockifier-compatible outputs:
//! - `TransactionExecutionInfo`
//! - `StateMaps`
//!
//! The goal is to keep `mc-block-production`'s executor loop readable by encapsulating the
//! "run rust-exec + format like blockifier + apply writes to CachedState" glue here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use blockifier::blockifier::transaction_executor::{
    TransactionExecutionOutput, TransactionExecutor, TransactionExecutorError, TransactionExecutorResult,
    BLOCK_STATE_ACCESS_ERR,
};
use blockifier::bouncer::{get_tx_weights, Bouncer, BouncerWeights};
use blockifier::execution::call_info::{CairoPrimitiveCounterMap, ExecutionSummary};
use blockifier::fee::receipt::TransactionReceipt;
use blockifier::state::cached_state::StateChangesKeys;
use blockifier::state::state_api::{StateReader as BlockifierStateReader, UpdatableState};
use blockifier::transaction::transaction_execution::Transaction;

use mp_convert::ToFelt;
use starknet_types_core::felt::Felt;

use crate::initialize_runtime_config;
use crate::integration::blockifier::{
    rust_execute_transaction_blockifier_output, rust_tx_diff_log_enabled, RustBlockifierOutput, RustExecStateAdapter,
    RustExecutionOutcome,
};
use crate::telemetry::hash_agg;
use crate::telemetry::storage_agg;
use crate::RustExecRuntimeConfig;

/// Result of running rust-exec in "shadow" mode for block production comparisons.
///
/// This runs the exact same rust-exec path as block production (same state adapter + same
/// formatting logic), but it does **not** apply writes to the `CachedState` and does **not**
/// touch the bouncer. Intended for `ExecutorMode::Both` debugging.
#[derive(Debug)]
pub struct RustShadowExecutionResult {
    pub outcome: RustExecutionOutcome,
    pub formatted: TransactionExecutorResult<TransactionExecutionOutput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustDeferredReason {
    Capacity,
    ResourceError,
    UnsupportedOrFailed,
}

#[derive(Debug, Clone, Default)]
pub struct RustPhaseState {
    pub first_tx_in_block: bool,
    pub projected_bouncer_weights: Option<BouncerWeights>,
}

impl RustPhaseState {
    /// Take the conservative value in every resource dimension while keeping the live transaction count.
    pub fn effective_bouncer_weights(&self, live_bouncer_weights: BouncerWeights) -> BouncerWeights {
        let Some(projected) = self.projected_bouncer_weights else {
            return live_bouncer_weights;
        };
        BouncerWeights {
            l1_gas: projected.l1_gas.max(live_bouncer_weights.l1_gas),
            message_segment_length: projected.message_segment_length.max(live_bouncer_weights.message_segment_length),
            n_events: projected.n_events.max(live_bouncer_weights.n_events),
            state_diff_size: projected.state_diff_size.max(live_bouncer_weights.state_diff_size),
            sierra_gas: projected.sierra_gas.max(live_bouncer_weights.sierra_gas),
            n_txs: live_bouncer_weights.n_txs,
            proving_gas: projected.proving_gas.max(live_bouncer_weights.proving_gas),
            receipt_l2_gas: projected.receipt_l2_gas.max(live_bouncer_weights.receipt_l2_gas),
        }
    }

    pub fn blockifier_reservation(&self, live_bouncer_weights: BouncerWeights) -> (BouncerWeights, BouncerWeights) {
        let effective = self.effective_bouncer_weights(live_bouncer_weights);
        let reserved =
            effective.checked_sub(live_bouncer_weights).expect("effective bouncer weights must dominate live weights");
        (effective, reserved)
    }

    pub fn absorb_blockifier_delta(
        &mut self,
        effective_before: BouncerWeights,
        live_before: BouncerWeights,
        live_after: BouncerWeights,
    ) {
        let delta = live_after.checked_sub(live_before).expect("Blockifier bouncer weights cannot decrease");
        let mut effective_after = effective_before.checked_add(delta).expect("effective bouncer weights overflowed");
        effective_after.n_txs = live_after.n_txs;
        self.projected_bouncer_weights = Some(effective_after);
    }
}

/// Temporarily reserves Rust's conservative projected headroom from Blockifier's public capacity.
pub struct ScopedBlockifierCapacity {
    bouncer: Arc<Mutex<Bouncer>>,
    original_capacity: BouncerWeights,
}

impl ScopedBlockifierCapacity {
    pub fn reserve(bouncer: Arc<Mutex<Bouncer>>, reserved: BouncerWeights) -> Option<Self> {
        let original_capacity = {
            let mut bouncer = bouncer.lock().expect("Bouncer lock poisoned");
            let original_capacity = bouncer.bouncer_config.block_max_capacity;
            bouncer.bouncer_config.block_max_capacity = original_capacity.checked_sub(reserved)?;
            original_capacity
        };
        Some(Self { bouncer, original_capacity })
    }
}

impl Drop for ScopedBlockifierCapacity {
    fn drop(&mut self) {
        self.bouncer.lock().expect("Bouncer lock poisoned").bouncer_config.block_max_capacity = self.original_capacity;
    }
}

#[derive(Debug)]
pub struct RustExecOutput {
    pub results: Vec<TransactionExecutorResult<TransactionExecutionOutput>>,
    pub executed_count: usize,
    pub block_full: bool,
    pub deferred_reason: Option<RustDeferredReason>,
}

struct RustExecOptions {
    apply_writes: bool,
    update_bouncer: bool,
}

fn rust_bouncer_delta<S: BlockifierStateReader>(
    state_reader: &S,
    bouncer: &blockifier::bouncer::Bouncer,
    tx_execution_summary: &ExecutionSummary,
    tx_state_changes_keys: &StateChangesKeys,
    tx_builtin_counters: &CairoPrimitiveCounterMap,
    receipt: &TransactionReceipt,
    versioned_constants: &blockifier::blockifier_versioned_constants::VersionedConstants,
) -> TransactionExecutorResult<BouncerWeights> {
    let marginal_state_changes_keys = tx_state_changes_keys.difference(&bouncer.state_changes_keys);
    let already_executed_class_hashes = bouncer.get_executed_class_hashes();
    let marginal_executed_class_hashes =
        tx_execution_summary.executed_class_hashes.difference(&already_executed_class_hashes).cloned().collect();
    let n_marginal_visited_storage_entries =
        tx_execution_summary.visited_storage_entries.difference(&bouncer.visited_storage_entries).count();

    let tx_weights = get_tx_weights(
        state_reader,
        &marginal_executed_class_hashes,
        n_marginal_visited_storage_entries,
        &receipt.resources,
        &marginal_state_changes_keys,
        versioned_constants,
        tx_builtin_counters,
        &bouncer.bouncer_config,
        receipt.gas.l2_gas,
    )
    .map_err(TransactionExecutorError::TransactionExecutionError)?;

    Ok(tx_weights.bouncer_weights)
}

fn log_hash_agg(tx_hash: Felt, outcome: &RustExecutionOutcome, hash_stats: hash_agg::HashAggSnapshot) {
    let outcome_label = match outcome {
        RustExecutionOutcome::Executed(_) => "Executed",
        RustExecutionOutcome::Skipped { .. } => "Skipped",
        RustExecutionOutcome::Failed(_) => "Failed",
    };

    tracing::debug!(
        "hash_total tx={:#x} outcome={} pedersen_calls={} pedersen_hits={} pedersen_misses={} \
poseidon_calls={} sn_keccak_calls={} sn_keccak_hits={} sn_keccak_misses={} key_cache_hits={} key_cache_misses={} \
ctx_reads_total={} ctx_read_cache_hits={} ctx_write_hits={} ctx_backend_reads={} cached_state_reads_total={} \
cached_state_cache_hits={} cached_state_cache_misses={}",
        tx_hash,
        outcome_label,
        hash_stats.pedersen_calls,
        hash_stats.pedersen_hits,
        hash_stats.pedersen_misses,
        hash_stats.poseidon_calls,
        hash_stats.sn_keccak_calls,
        hash_stats.sn_keccak_hits,
        hash_stats.sn_keccak_misses,
        hash_stats.key_cache_hits,
        hash_stats.key_cache_misses,
        hash_stats.ctx_reads_total,
        hash_stats.ctx_read_cache_hits,
        hash_stats.ctx_write_hits,
        hash_stats.ctx_backend_reads,
        0,
        0,
        0
    );

    let total_unique = hash_stats.pedersen_inputs + hash_stats.poseidon_inputs + hash_stats.sn_keccak_inputs;
    tracing::debug!(
        "hash_unique tx={:#x} pedersen_inputs={} poseidon_inputs={} sn_keccak_inputs={} total_unique={}",
        tx_hash,
        hash_stats.pedersen_inputs,
        hash_stats.poseidon_inputs,
        hash_stats.sn_keccak_inputs,
        total_unique
    );
}

fn log_storage_agg(tx_hash: Felt, outcome: &RustExecutionOutcome, ctx_stats: storage_agg::StorageAggSnapshot) {
    let outcome_label = match outcome {
        RustExecutionOutcome::Executed(_) => "Executed",
        RustExecutionOutcome::Skipped { .. } => "Skipped",
        RustExecutionOutcome::Failed(_) => "Failed",
    };

    tracing::debug!(
        "storage_total tx={:#x} outcome={} ctx_reads_total={} ctx_read_cache_hits={} \
ctx_write_cache_hits={} ctx_backend_reads={} ctx_read_cache_us={} ctx_write_cache_us={} \
ctx_backend_us={} ctx_writes_total={} ctx_write_us={} cached_state_reads_total={} \
cached_state_cache_hits={} cached_state_cache_misses={} cached_state_read_us={} \
cached_state_writes_total={} cached_state_write_us={} layered_reads_total={} \
layered_cache_hits={} layered_cache_misses={} layered_read_us={} backend_reads_total={} \
backend_read_us={}",
        tx_hash,
        outcome_label,
        ctx_stats.ctx_reads_total,
        ctx_stats.ctx_read_cache_hits,
        ctx_stats.ctx_write_cache_hits,
        ctx_stats.ctx_backend_reads,
        ctx_stats.ctx_read_cache_us,
        ctx_stats.ctx_write_cache_us,
        ctx_stats.ctx_backend_us,
        ctx_stats.ctx_writes_total,
        ctx_stats.ctx_write_us,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
    );

    let total_unique_reads = ctx_stats
        .ctx_read_cache_unique
        .saturating_add(ctx_stats.ctx_write_cache_unique)
        .saturating_add(ctx_stats.ctx_backend_unique)
        .saturating_add(0)
        .saturating_add(0)
        .saturating_add(0);
    let total_unique_writes = ctx_stats.ctx_write_unique;

    tracing::debug!(
        "storage_unique tx={:#x} ctx_read_cache_unique={} ctx_write_cache_unique={} \
ctx_backend_unique={} ctx_write_unique={} cached_state_unique_reads={} \
cached_state_unique_writes={} layered_unique_reads={} backend_unique_reads={} \
total_unique_reads={} total_unique_writes={}",
        tx_hash,
        ctx_stats.ctx_read_cache_unique,
        ctx_stats.ctx_write_cache_unique,
        ctx_stats.ctx_backend_unique,
        ctx_stats.ctx_write_unique,
        0,
        0,
        0,
        0,
        total_unique_reads,
        total_unique_writes,
    );
}

fn execute_settle_trade_v3_internal<S: BlockifierStateReader + Send + Sync + 'static>(
    executor: &mut TransactionExecutor<S>,
    txs: &[Transaction],
    execution_deadline: Option<Instant>,
    phase_state: Option<&mut RustPhaseState>,
    options: RustExecOptions,
) -> Vec<RustBlockifierOutput> {
    let mut results: Vec<RustBlockifierOutput> = Vec::with_capacity(txs.len());

    if options.apply_writes {
        let mut phase_state =
            if options.update_bouncer { Some(phase_state.expect("rust phase state missing")) } else { None };
        let mut block_state = executor.block_state.take().expect(BLOCK_STATE_ACCESS_ERR);
        for tx in txs {
            if let Some(deadline) = execution_deadline {
                if Instant::now() > deadline {
                    tracing::debug!("execution_timeout");
                    break;
                }
            }

            let tx_hash = Transaction::tx_hash(tx).to_felt();

            // C-022: Pre-execution bouncer pre-check.
            // Before executing, verify the block can fit the minimum guaranteed delta
            // (`n_txs = 1`). If even this cannot fit, skip execution
            // entirely — the tx remains deferred in the caller's suffix.
            if options.update_bouncer {
                let ps = phase_state.as_deref_mut().expect("rust phase state missing");
                let bouncer = executor.bouncer.lock().expect("Bouncer lock poisoned");
                let projected_current = ps.effective_bouncer_weights(*bouncer.get_bouncer_weights());
                let min_tx_delta = BouncerWeights { n_txs: 1, ..BouncerWeights::empty() };
                if let Some(projected_min) = projected_current.checked_add(min_tx_delta) {
                    if !bouncer.bouncer_config.has_room(projected_min) {
                        tracing::info!(tx_hash = format_args!("{:#x}", tx_hash),
                            projected_current_bouncer_weights = ?projected_current,
                            min_tx_delta = ?min_tx_delta,
                            "tx_deferred_precheck_block_full"
                        );
                        break;
                    }
                }
                // Bouncer lock dropped here; execution proceeds below.
            }

            crate::core::storage::reset_key_derivation_cache();
            let rust_state = RustExecStateAdapter::new(&block_state);
            if hash_agg::enabled() {
                hash_agg::reset();
            }
            if storage_agg::enabled() {
                storage_agg::reset();
            }
            let mut output =
                rust_execute_transaction_blockifier_output(&rust_state, tx, &executor.block_context, tx_hash);
            if hash_agg::enabled() {
                let hash_stats = hash_agg::snapshot();
                log_hash_agg(tx_hash, &output.outcome, hash_stats);
            }
            if storage_agg::enabled() {
                let ctx_stats = storage_agg::snapshot();
                log_storage_agg(tx_hash, &output.outcome, ctx_stats);
            }
            let output_failed = output.output.is_err();
            if let Ok((execution_info, maps)) = output.output.as_mut() {
                let block_number = executor.block_context.block_info().block_number.0;
                if rust_tx_diff_log_enabled(block_number) {
                    for ((contract_address, storage_key), value) in &maps.storage {
                        if *value != Felt::ZERO {
                            continue;
                        }
                        let parent_value = block_state.state.get_storage_at(*contract_address, *storage_key);
                        tracing::info!(
                            target: "RUST_EXEC",
                            "zero_write_before_cached_state_apply block_number={} tx_hash={:#x} contract_address={:#x} storage_key={:#x} parent_value={:?}",
                            block_number,
                            tx_hash,
                            contract_address.to_felt(),
                            storage_key.to_felt(),
                            parent_value,
                        );
                    }
                }
                if options.update_bouncer {
                    let tx_state_changes_keys = maps.keys();
                    let tx_execution_summary = execution_info.summarize(executor.block_context.versioned_constants());
                    let tx_builtin_counters = execution_info.summarize_builtins();
                    let mut bouncer = executor.bouncer.lock().expect("Bouncer lock poisoned");
                    let phase_state = phase_state.as_deref_mut().expect("rust phase state missing");
                    let projected_current = phase_state.effective_bouncer_weights(*bouncer.get_bouncer_weights());
                    let tx_projected_delta = rust_bouncer_delta(
                        &block_state,
                        &bouncer,
                        &tx_execution_summary,
                        &tx_state_changes_keys,
                        &tx_builtin_counters,
                        &execution_info.receipt,
                        executor.block_context.versioned_constants(),
                    )
                    .expect("failed to compute rust tx bouncer delta");
                    let projected_next = projected_current
                        .checked_add(tx_projected_delta)
                        .expect("Rust projected bouncer weights overflowed");
                    // C-022: Post-execution bouncer check. If the full delta doesn't fit,
                    // the tx is NOT committed — no writes applied, no result pushed.
                    // The tx remains deferred in the caller's suffix (pending_routed.rust_batch).
                    if !bouncer.bouncer_config.has_room(projected_next) {
                        tracing::info!(tx_hash = format_args!("{:#x}", tx_hash),
                            tx_projected_delta = ?tx_projected_delta,
                            projected_current_bouncer_weights = ?projected_current,
                            projected_next_bouncer_weights = ?projected_next,
                            "tx_deferred_post_exec_block_full"
                        );
                        break;
                    }
                    if bouncer
                        .try_update(
                            &block_state,
                            &tx_state_changes_keys,
                            &tx_execution_summary,
                            &tx_builtin_counters,
                            &execution_info.receipt.resources,
                            executor.block_context.versioned_constants(),
                            execution_info.receipt.gas.l2_gas,
                        )
                        .is_err()
                    {
                        tracing::info!(
                            tx_hash = format_args!("{:#x}", tx_hash),
                            "tx_deferred_bouncer_try_update_rejected"
                        );
                        break;
                    }
                    phase_state.first_tx_in_block = false;
                    phase_state.projected_bouncer_weights = Some(projected_next);
                    let current_weights = bouncer.get_bouncer_weights();
                    tracing::debug!(tx_hash = format_args!("{:#x}", tx_hash),
                        tx_summary = ?tx_execution_summary,
                        tx_builtin_counters = ?tx_builtin_counters,
                        tx_receipt_resources = ?execution_info.receipt.resources,
                        projected_tx_bouncer_delta = ?tx_projected_delta,
                        current_bouncer_weights = ?current_weights,
                        projected_bouncer_weights = ?projected_next,
                        "bouncer_after_tx"
                    );
                }

                block_state.apply_writes(maps, &HashMap::new());
            }

            results.push(output);
            if output_failed {
                break;
            }
        }

        executor.block_state = Some(block_state);
        return results;
    }

    let block_state = executor.block_state.as_ref().expect(BLOCK_STATE_ACCESS_ERR);
    for tx in txs {
        if let Some(deadline) = execution_deadline {
            if Instant::now() > deadline {
                tracing::debug!("execution_timeout");
                break;
            }
        }

        let tx_hash = Transaction::tx_hash(tx).to_felt();
        crate::core::storage::reset_key_derivation_cache();
        let rust_state = RustExecStateAdapter::new(block_state);
        if hash_agg::enabled() {
            hash_agg::reset();
        }
        if storage_agg::enabled() {
            storage_agg::reset();
        }
        let output = rust_execute_transaction_blockifier_output(&rust_state, tx, &executor.block_context, tx_hash);
        if hash_agg::enabled() {
            let hash_stats = hash_agg::snapshot();
            log_hash_agg(tx_hash, &output.outcome, hash_stats);
        }
        if storage_agg::enabled() {
            let ctx_stats = storage_agg::snapshot();
            log_storage_agg(tx_hash, &output.outcome, ctx_stats);
        }
        results.push(output);
    }

    results
}

/// Execute rust-exec supported transactions, producing Blockifier-compatible outputs.
///
/// Semantics:
/// - Runs rust-exec against the executor's current `CachedState`.
/// - Applies the rust-exec produced state diff to that `CachedState`.
/// - Produces a `TransactionExecutionInfo` (receipt/call-info) and `StateMaps` (tx diff).
/// - Uses selector-specific fee/resource bridges when rust-exec requires them
///   (for example `settle_trade_v3`).
/// - Updates the executor bouncer using the same resource inputs Blockifier uses.
///
/// If the bouncer cannot fit a tx, execution stops early and returns fewer results than `txs.len()`
/// (matching Blockifier behavior).
pub fn execute_txs_settle_trade_v3<S: BlockifierStateReader + Send + Sync + 'static>(
    executor: &mut TransactionExecutor<S>,
    txs: &[Transaction],
    execution_deadline: Option<Instant>,
    phase_state: &mut RustPhaseState,
) -> Vec<TransactionExecutorResult<TransactionExecutionOutput>> {
    execute_settle_trade_v3_internal(
        executor,
        txs,
        execution_deadline,
        Some(phase_state),
        RustExecOptions { apply_writes: true, update_bouncer: true },
    )
    .into_iter()
    .map(|item| item.output)
    .collect()
}

/// Execute a single `settle_trade_v3` transaction in "shadow" mode:
/// - Runs rust-exec against the current executor `CachedState` (read-only).
/// - Produces Blockifier-compatible `(TransactionExecutionInfo, StateMaps)` from the rust outcome.
/// - Does **not** apply writes to the block state.
/// - Does **not** update bouncer weights.
pub fn execute_settle_trade_v3_shadow<S: BlockifierStateReader + Send + Sync + 'static>(
    executor: &mut TransactionExecutor<S>,
    tx: &Transaction,
) -> RustShadowExecutionResult {
    let mut results = execute_settle_trade_v3_internal(
        executor,
        std::slice::from_ref(tx),
        None,
        None,
        RustExecOptions { apply_writes: false, update_bouncer: false },
    );
    let output = results.pop().unwrap_or_else(|| RustBlockifierOutput {
        outcome: RustExecutionOutcome::Skipped {
            reason: crate::integration::blockifier::RustExecutionSkipReason::NonInvokeTransaction,
            tx_hash: Felt::ZERO,
            block_timestamp: 0,
        },
        output: Err(TransactionExecutorError::TransactionExecutionError(
            blockifier::transaction::errors::TransactionExecutionError::StateError(
                blockifier::state::errors::StateError::StateReadError(
                    "rust exec shadow produced no result".to_string(),
                ),
            ),
        )),
    });

    RustShadowExecutionResult { outcome: output.outcome, formatted: output.output }
}

pub fn execute_txns<S: BlockifierStateReader + Send + Sync + 'static>(
    executor: &mut TransactionExecutor<S>,
    txs: &[Transaction],
    execution_deadline: Option<Instant>,
    runtime_cfg: &RustExecRuntimeConfig,
    phase_state: &mut RustPhaseState,
) -> RustExecOutput {
    initialize_runtime_config(runtime_cfg.clone());

    let mut results = execute_txs_settle_trade_v3(executor, txs, execution_deadline, phase_state);
    let (executed_count, block_full, deferred_reason) = classify_rust_results(&results, txs.len());
    if matches!(deferred_reason, Some(RustDeferredReason::UnsupportedOrFailed)) {
        results.truncate(executed_count);
    }

    RustExecOutput { results, executed_count, block_full, deferred_reason }
}

fn classify_rust_results<T, E>(
    results: &[Result<T, E>],
    input_len: usize,
) -> (usize, bool, Option<RustDeferredReason>) {
    let executed_count = results.iter().take_while(|result| result.is_ok()).count();
    if executed_count < results.len() {
        return (executed_count, false, Some(RustDeferredReason::UnsupportedOrFailed));
    }

    let block_full = executed_count < input_len;
    (executed_count, block_full, block_full.then_some(RustDeferredReason::Capacity))
}

#[cfg(test)]
mod tests {
    use super::{classify_rust_results, RustDeferredReason, RustPhaseState, ScopedBlockifierCapacity};
    use blockifier::bouncer::{Bouncer, BouncerConfig, BouncerWeights};
    use starknet_api::execution_resources::GasAmount;
    use std::sync::{Arc, Mutex};

    #[test]
    fn rust_result_classification_stops_before_first_error() {
        let results = [Ok(()), Ok(()), Err(()), Ok(())];
        assert_eq!(classify_rust_results(&results, 4), (2, false, Some(RustDeferredReason::UnsupportedOrFailed)));
    }

    #[test]
    fn projected_weights_use_live_shared_transaction_count() {
        let phase_state = RustPhaseState {
            projected_bouncer_weights: Some(BouncerWeights {
                n_txs: 2,
                l1_gas: 91,
                message_segment_length: 92,
                n_events: 93,
                state_diff_size: 99,
                sierra_gas: GasAmount(10),
                proving_gas: GasAmount(95),
                receipt_l2_gas: GasAmount(96),
                ..BouncerWeights::empty()
            }),
            ..Default::default()
        };
        let live = BouncerWeights {
            n_txs: 7,
            l1_gas: 11,
            message_segment_length: 12,
            n_events: 13,
            state_diff_size: 14,
            sierra_gas: GasAmount(20),
            proving_gas: GasAmount(15),
            receipt_l2_gas: GasAmount(16),
        };

        let effective = phase_state.effective_bouncer_weights(live);

        assert_eq!(effective.n_txs, 7);
        assert_eq!(effective.l1_gas, 91);
        assert_eq!(effective.message_segment_length, 92);
        assert_eq!(effective.n_events, 93);
        assert_eq!(effective.state_diff_size, 99);
        assert_eq!(effective.sierra_gas, GasAmount(20));
        assert_eq!(effective.proving_gas, GasAmount(95));
        assert_eq!(effective.receipt_l2_gas, GasAmount(96));
    }

    #[test]
    fn blockifier_delta_advances_the_conservative_projection() {
        let mut phase_state = RustPhaseState::default();
        let live_before = BouncerWeights {
            n_txs: 1,
            l1_gas: 1,
            message_segment_length: 2,
            n_events: 3,
            state_diff_size: 10,
            sierra_gas: GasAmount(20),
            proving_gas: GasAmount(30),
            receipt_l2_gas: GasAmount(40),
        };
        let effective_before = BouncerWeights {
            n_txs: 1,
            l1_gas: 11,
            message_segment_length: 12,
            n_events: 13,
            state_diff_size: 30,
            sierra_gas: GasAmount(100),
            proving_gas: GasAmount(110),
            receipt_l2_gas: GasAmount(120),
        };
        let live_after = BouncerWeights {
            n_txs: 3,
            l1_gas: 3,
            message_segment_length: 5,
            n_events: 7,
            state_diff_size: 18,
            sierra_gas: GasAmount(25),
            proving_gas: GasAmount(36),
            receipt_l2_gas: GasAmount(47),
        };

        phase_state.absorb_blockifier_delta(effective_before, live_before, live_after);

        assert_eq!(
            phase_state.projected_bouncer_weights,
            Some(BouncerWeights {
                n_txs: 3,
                l1_gas: 13,
                message_segment_length: 15,
                n_events: 17,
                state_diff_size: 38,
                sierra_gas: GasAmount(105),
                proving_gas: GasAmount(116),
                receipt_l2_gas: GasAmount(127),
            })
        );
    }

    #[test]
    fn scoped_blockifier_capacity_restores_the_original_limit() {
        let max = BouncerWeights {
            n_txs: 10,
            l1_gas: 100,
            message_segment_length: 100,
            n_events: 100,
            state_diff_size: 100,
            sierra_gas: GasAmount(1_000),
            proving_gas: GasAmount(1_000),
            receipt_l2_gas: GasAmount(1_000),
        };
        let bouncer = Arc::new(Mutex::new(Bouncer::new(BouncerConfig {
            block_max_capacity: max,
            builtin_weights: Default::default(),
        })));
        let reserved = BouncerWeights {
            n_txs: 0,
            l1_gas: 10,
            message_segment_length: 11,
            n_events: 12,
            state_diff_size: 20,
            sierra_gas: GasAmount(300),
            proving_gas: GasAmount(301),
            receipt_l2_gas: GasAmount(302),
        };

        {
            let _guard = ScopedBlockifierCapacity::reserve(bouncer.clone(), reserved).expect("reservation fits");
            let reduced = bouncer.lock().unwrap().bouncer_config.block_max_capacity;
            assert_eq!(reduced.l1_gas, 90);
            assert_eq!(reduced.message_segment_length, 89);
            assert_eq!(reduced.n_events, 88);
            assert_eq!(reduced.state_diff_size, 80);
            assert_eq!(reduced.sierra_gas, GasAmount(700));
            assert_eq!(reduced.proving_gas, GasAmount(699));
            assert_eq!(reduced.receipt_l2_gas, GasAmount(698));
        }

        assert_eq!(bouncer.lock().unwrap().bouncer_config.block_max_capacity, max);
        let too_large = BouncerWeights { state_diff_size: 101, ..BouncerWeights::empty() };
        assert!(ScopedBlockifierCapacity::reserve(bouncer.clone(), too_large).is_none());
        assert_eq!(bouncer.lock().unwrap().bouncer_config.block_max_capacity, max);
    }

    #[test]
    fn scoped_blockifier_capacity_restores_the_limit_during_unwind() {
        let max = BouncerWeights { state_diff_size: 100, ..BouncerWeights::max() };
        let bouncer = Arc::new(Mutex::new(Bouncer::new(BouncerConfig {
            block_max_capacity: max,
            builtin_weights: Default::default(),
        })));
        let reserved = BouncerWeights { state_diff_size: 20, ..BouncerWeights::empty() };

        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
            let bouncer = bouncer.clone();
            move || {
                let _guard = ScopedBlockifierCapacity::reserve(bouncer, reserved).expect("reservation fits");
                panic!("simulate Blockifier failure");
            }
        }));

        assert!(unwound.is_err());
        assert_eq!(bouncer.lock().unwrap().bouncer_config.block_max_capacity, max);
    }
}
