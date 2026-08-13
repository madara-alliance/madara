/// T-053: Blocking re-execution worker.
///
/// This runs on the tokio blocking thread pool (via `mp_utils::spawn_blocking`).
/// It reconstructs the block X execution context from scratch, runs Blockifier-only
/// execution in deterministic ordered chunks, and checks for cancellation between chunks.
use super::{ReexecExecutedTxArtifacts, ReexecRequest, ReexecResult, ReexecWorkerOutcome, REEXEC_CHUNK_SIZE};
use anyhow::Context as _;
use mc_exec::LayeredStateAdapter;
use mp_convert::{Felt, ToFelt};
use mp_receipt::from_blockifier_execution_info;
use mp_state_update::{ClassUpdateItem, StateDiff, TransactionStateUpdate};
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

/// Entry point for a single blocking re-execution job.
///
/// Called from the dispatcher via `mp_utils::spawn_blocking`. This function never
/// touches the Tokio runtime directly and must not call `.await`.
pub fn run_blockifier_reexec(req: ReexecRequest, cancel: CancellationToken) -> anyhow::Result<ReexecWorkerOutcome> {
    // Fast-path: cancelled before we even start.
    if cancel.is_cancelled() {
        return Ok(ReexecWorkerOutcome::Cancelled { epoch: req.epoch, block_n: req.block_n });
    }

    // Build parent state for re-execution (C-007C / C-007F).
    //
    // Fast path: use LayeredStateAdapter::new(backend) only when the target block is
    // the direct child of the confirmed base. This is the only case where the DB's
    // latest_confirmed + 1 is guaranteed to equal the target.
    //
    // All other cases (runahead, genesis with gap) go through new_for_reexec() which
    // validates overlay contiguity and fails closed on missing overlays.
    let is_direct_child = match req.confirmed_base_block_n {
        Some(c) => req.block_n == c + 1,
        None => req.block_n == 0,
    };

    let overlay_count = req.parent_overlays.len();
    let overlay_first_block_n = req.parent_overlays.first().map(|overlay| overlay.block_n);
    let overlay_last_block_n = req.parent_overlays.last().map(|overlay| overlay.block_n);

    let state_adapter = if is_direct_child && req.parent_overlays.is_empty() {
        // Direct child of confirmed base — no overlays needed.
        tracing::debug!(
            block_n = req.block_n,
            confirmed_base_block_n = ?req.confirmed_base_block_n,
            parent_state_source = "confirmed_db_direct_parent",
            "comparator_reexec_parent_state_selected"
        );
        LayeredStateAdapter::new(req.backend.clone())
            .context("Creating LayeredStateAdapter for re-execution worker (direct child)")?
    } else {
        // Runahead or non-trivial path: build synthetic parent from confirmed base + overlays.
        // new_for_reexec() validates contiguity and fails closed if overlays are missing.
        tracing::debug!(
            block_n = req.block_n,
            confirmed_base_block_n = ?req.confirmed_base_block_n,
            parent_state_source = "synthetic_parent_with_overlays",
            overlay_count,
            overlay_first_block_n = ?overlay_first_block_n,
            overlay_last_block_n = ?overlay_last_block_n,
            "comparator_reexec_parent_state_selected"
        );
        tracing::debug!(
            block_n = req.block_n,
            confirmed_base = ?req.confirmed_base_block_n,
            overlay_count,
            is_direct_child,
            "re-exec worker building synthetic parent state"
        );
        LayeredStateAdapter::new_for_reexec(
            req.backend.clone(),
            req.confirmed_base_block_n,
            req.block_n,
            req.parent_overlays.clone(),
        )
        .context("Creating synthetic LayeredStateAdapter for re-execution under runahead")?
    };

    // Verify the adapter targets the expected block.
    let adapter_block_n = state_adapter.block_n();
    anyhow::ensure!(
        adapter_block_n == req.block_n,
        "Re-execution state adapter targets block {adapter_block_n} but expected block {}. \
         confirmed_base={:?}, overlay_count={}",
        req.block_n,
        req.confirmed_base_block_n,
        overlay_count,
    );

    // Reconstruct executor with block_n-10 hash entry (Starknet protocol requirement).
    let backend = req.backend.clone();
    let exec_ctx = req.exec_ctx.clone();
    let block_n = req.block_n;
    let mut executor = crate::util::create_executor_with_block_n_min_10(
        &backend,
        &exec_ctx,
        state_adapter,
        |bn| get_block_n_min_10_hash(&backend, bn),
        None, // use backend's current chain config
    )
    .context("Creating executor for re-execution worker")?;

    // Execute transactions in chunks, checking cancellation between chunks.
    // C-013: Preserve per-tx execution artifacts for BRE-backed external promotion.
    let mut per_tx_artifacts: Vec<ReexecExecutedTxArtifacts> = Vec::with_capacity(req.txs.len());
    let mut tx_idx = 0;
    let mut deployed_in_block: HashSet<Felt> = HashSet::new();
    let mut per_tx_complete = true;

    for chunk in req.txs.chunks(REEXEC_CHUNK_SIZE) {
        if cancel.is_cancelled() {
            return Ok(ReexecWorkerOutcome::Cancelled { epoch: req.epoch, block_n });
        }
        let results = executor.execute_txs(chunk, /* deadline */ None);
        let n_processed = results.len();
        for (i, result) in results.into_iter().enumerate() {
            let blockifier_tx = &req.txs[tx_idx + i];
            match result {
                Ok((execution_info, state_maps)) => {
                    let receipt = from_blockifier_execution_info(&execution_info, blockifier_tx);
                    let tx_state_update = convert_bre_state_maps(state_maps, &mut deployed_in_block, &req.backend)?;
                    per_tx_artifacts.push(ReexecExecutedTxArtifacts { receipt, tx_state_update });
                }
                Err(e) => {
                    // Per-tx failure: mark as incomplete. Block-level state_diff is still
                    // produced by finalize(). Per-tx artifacts will not be usable.
                    tracing::warn!(block_n, tx_idx = tx_idx + i, "BRE per-tx execution error: {e:#}");
                    per_tx_complete = false;
                }
            }
        }
        tx_idx += n_processed;
    }

    // If per-tx artifacts are incomplete, keep the canonical included prefix that we do have.
    // The caller must replay the speculative suffix instead of silently keeping EB-backed rows.
    if !per_tx_complete || per_tx_artifacts.len() != req.txs.len() {
        tracing::warn!(
            block_n,
            expected = req.txs.len(),
            got = per_tx_artifacts.len(),
            "BRE per-tx artifacts incomplete — stop-path must split canonical prefix from replay suffix"
        );
    }

    if cancel.is_cancelled() {
        return Ok(ReexecWorkerOutcome::Cancelled { epoch: req.epoch, block_n });
    }

    // Finalize executor to extract the complete state diff and bouncer weights.
    let summary = executor.finalize().context("Finalizing re-execution executor")?;

    let migration_v2_hashes: HashSet<Felt> =
        summary.compiled_class_hashes_for_migration.iter().map(|(v2_hash, _)| v2_hash.0).collect();

    let state_diff = StateDiff::from_blockifier(
        summary.state_diff,
        &migration_v2_hashes,
        &req.deployed_contracts_set,
        req.old_declared_contracts,
    );

    Ok(ReexecWorkerOutcome::Completed(ReexecResult {
        epoch: req.epoch,
        state_diff,
        exec_resources: summary.bouncer_weights,
        per_tx: per_tx_artifacts,
    }))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert per-tx Blockifier `StateMaps` into `TransactionStateUpdate` (C-013).
///
/// Handles deployed vs replaced classification using an accumulating set
/// (same pattern as the normal append path in `append_batch`).
/// `declared_classes` is left empty — the caller merges it from original metadata.
fn convert_bre_state_maps(
    state_maps: blockifier::state::cached_state::StateMaps,
    deployed_in_block: &mut HashSet<Felt>,
    backend: &std::sync::Arc<mc_db::MadaraBackend>,
) -> anyhow::Result<TransactionStateUpdate> {
    let nonces = state_maps
        .nonces
        .into_iter()
        .map(|(contract_addr, nonce)| (contract_addr.to_felt(), nonce.to_felt()))
        .collect();

    let storage_diffs = state_maps
        .storage
        .into_iter()
        .map(|((contract_addr, key), value)| ((contract_addr.to_felt(), key.to_felt()), value))
        .collect();

    let contract_class_hashes = state_maps
        .class_hashes
        .into_iter()
        .map(|(contract_addr, class_hash)| {
            let addr_felt = contract_addr.to_felt();
            let entry = if !deployed_in_block.contains(&addr_felt)
                && !backend.view_on_latest_confirmed().is_contract_deployed(&addr_felt)?
            {
                deployed_in_block.insert(addr_felt);
                ClassUpdateItem::DeployedContract(class_hash.to_felt())
            } else {
                ClassUpdateItem::ReplacedClass(class_hash.to_felt())
            };
            Ok((addr_felt, entry))
        })
        .collect::<anyhow::Result<_>>()?;

    Ok(TransactionStateUpdate { nonces, storage_diffs, contract_class_hashes, declared_classes: Default::default() })
}

fn get_block_n_min_10_hash(
    backend: &std::sync::Arc<mc_db::MadaraBackend>,
    block_n: u64,
) -> anyhow::Result<Option<(u64, Felt)>> {
    let Some(block_n_min_10) = block_n.checked_sub(10) else {
        return Ok(None);
    };
    if let Some(view) = backend.block_view_on_confirmed(block_n_min_10) {
        let block_hash = view.get_block_info().context("Getting block hash for block_n-10")?.block_hash;
        Ok(Some((block_n_min_10, block_hash)))
    } else {
        anyhow::bail!("Cannot fetch block #{block_n_min_10} hash (required for block_n-10 context)")
    }
}
