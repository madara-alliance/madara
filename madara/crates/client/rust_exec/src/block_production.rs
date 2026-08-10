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
use std::time::Instant;

use blockifier::blockifier::transaction_executor::{
    TransactionExecutionOutput, TransactionExecutor, TransactionExecutorError, TransactionExecutorResult,
    BLOCK_STATE_ACCESS_ERR,
};
use blockifier::bouncer::{get_tx_weights, BouncerWeights};
use blockifier::execution::call_info::{CairoPrimitiveCounterMap, ExecutionSummary};
use blockifier::fee::resources::TransactionResources;
use blockifier::state::cached_state::StateChangesKeys;
use blockifier::state::state_api::{StateReader as BlockifierStateReader, UpdatableState};
use blockifier::transaction::transaction_execution::Transaction;

use mp_convert::ToFelt;
use starknet_api::execution_resources::GasAmount;
use starknet_types_core::felt::Felt;

use crate::blockifier_integration::{
    rust_execute_transaction_blockifier_output, RustBlockifierOutput, RustExecStateAdapter, RustExecutionOutcome,
};
use crate::hash_agg;
use crate::initialize_runtime_config;
use crate::storage_agg;
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
}

#[derive(Debug, Clone, Default)]
pub struct RustPhaseState {
    pub first_tx_in_block: bool,
    pub projected_bouncer_weights: Option<BouncerWeights>,
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
    tx_resources: &TransactionResources,
    versioned_constants: &blockifier::blockifier_versioned_constants::VersionedConstants,
    receipt_l2_gas: GasAmount,
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
        tx_resources,
        &marginal_state_changes_keys,
        versioned_constants,
        tx_builtin_counters,
        &bouncer.bouncer_config,
        receipt_l2_gas,
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
        "rust_exec_hash_total tx={:#x} outcome={} pedersen_calls={} pedersen_hits={} pedersen_misses={} \
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
        "rust_exec_hash_unique tx={:#x} pedersen_inputs={} poseidon_inputs={} sn_keccak_inputs={} total_unique={}",
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
        "rust_exec_storage_total tx={:#x} outcome={} ctx_reads_total={} ctx_read_cache_hits={} \
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
        "rust_exec_storage_unique tx={:#x} ctx_read_cache_unique={} ctx_write_cache_unique={} \
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
                    tracing::debug!("Rust-exec execution timed out.");
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
                let projected_current = ps.projected_bouncer_weights.unwrap_or_else(|| *bouncer.get_bouncer_weights());
                let min_tx_delta = BouncerWeights { n_txs: 1, ..BouncerWeights::empty() };
                if let Some(projected_min) = projected_current.checked_add(min_tx_delta) {
                    if !bouncer.bouncer_config.has_room(projected_min) {
                        tracing::info!(
                            tx_hash = format_args!("{:#x}", tx_hash),
                            projected_current_bouncer_weights = ?projected_current,
                            min_tx_delta = ?min_tx_delta,
                            "rust_exec_tx_deferred_precheck_block_full"
                        );
                        break;
                    }
                }
                // Bouncer lock dropped here; execution proceeds below.
            }

            crate::storage::reset_key_derivation_cache();
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
            if let Ok((execution_info, maps)) = output.output.as_mut() {
                if options.update_bouncer {
                    let tx_state_changes_keys = maps.keys();
                    let tx_execution_summary = execution_info.summarize(executor.block_context.versioned_constants());
                    let tx_builtin_counters = execution_info.summarize_builtins();
                    let mut bouncer = executor.bouncer.lock().expect("Bouncer lock poisoned");
                    let phase_state = phase_state.as_deref_mut().expect("rust phase state missing");
                    let projected_current =
                        phase_state.projected_bouncer_weights.unwrap_or_else(|| *bouncer.get_bouncer_weights());
                    let tx_projected_delta = rust_bouncer_delta(
                        &block_state,
                        &bouncer,
                        &tx_execution_summary,
                        &tx_state_changes_keys,
                        &tx_builtin_counters,
                        &execution_info.receipt.resources,
                        executor.block_context.versioned_constants(),
                        execution_info.receipt.gas.l2_gas,
                    )
                    .expect("failed to compute rust tx bouncer delta");
                    let projected_next = projected_current
                        .checked_add(tx_projected_delta)
                        .expect("Rust projected bouncer weights overflowed");
                    // C-022: Post-execution bouncer check. If the full delta doesn't fit,
                    // the tx is NOT committed — no writes applied, no result pushed.
                    // The tx remains deferred in the caller's suffix (pending_routed.rust_batch).
                    if !bouncer.bouncer_config.has_room(projected_next) {
                        tracing::info!(
                            tx_hash = format_args!("{:#x}", tx_hash),
                            tx_projected_delta = ?tx_projected_delta,
                            projected_current_bouncer_weights = ?projected_current,
                            projected_next_bouncer_weights = ?projected_next,
                            "rust_exec_tx_deferred_post_exec_block_full"
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
                            "rust_exec_tx_deferred_bouncer_try_update_rejected"
                        );
                        break;
                    }
                    phase_state.first_tx_in_block = false;
                    phase_state.projected_bouncer_weights = Some(projected_next);
                    let current_weights = bouncer.get_bouncer_weights();
                    tracing::debug!(
                        tx_hash = format_args!("{:#x}", tx_hash),
                        tx_summary = ?tx_execution_summary,
                        tx_builtin_counters = ?tx_builtin_counters,
                        tx_receipt_resources = ?execution_info.receipt.resources,
                        projected_tx_bouncer_delta = ?tx_projected_delta,
                        current_bouncer_weights = ?current_weights,
                        projected_bouncer_weights = ?projected_next,
                        "rust_exec_bouncer_after_tx"
                    );
                }

                block_state.apply_writes(maps, &HashMap::new());
            }

            results.push(output);
        }

        executor.block_state = Some(block_state);
        return results;
    }

    let block_state = executor.block_state.as_ref().expect(BLOCK_STATE_ACCESS_ERR);
    for tx in txs {
        if let Some(deadline) = execution_deadline {
            if Instant::now() > deadline {
                tracing::debug!("Rust-exec execution timed out.");
                break;
            }
        }

        let tx_hash = Transaction::tx_hash(tx).to_felt();
        crate::storage::reset_key_derivation_cache();
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
            reason: crate::blockifier_integration::RustExecutionSkipReason::NonInvokeTransaction,
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

    let results = execute_txs_settle_trade_v3(executor, txs, execution_deadline, phase_state);
    let executed_count = results.len();
    let block_full = executed_count < txs.len();

    RustExecOutput {
        results,
        executed_count,
        block_full,
        deferred_reason: block_full.then_some(RustDeferredReason::Capacity),
    }
}
