use super::*;

struct CanonicalizationTaskContext {
    reexec_parent_state: ReexecParentStateSnapshot,
    dispatcher: Option<ReexecDispatcherHandle>,
    reexec_epoch: u64,
    metrics: Arc<BlockProductionMetrics>,
    no_charge_fee: bool,
    ignore_fee_token_mismatch: bool,
    ignored_storage_mismatch_canonical_source: RustExecCanonicalSource,
    internal_capacity: usize,
    #[cfg(test)]
    force_comparator_error: bool,
    #[cfg(test)]
    comparator_reexec_tx_limit: Option<usize>,
    #[cfg(test)]
    optimistic_pipeline_notifications: Option<mpsc::UnboundedSender<OptimisticPipelineNotification>>,
    #[cfg(test)]
    comparator_gate: Option<Arc<tokio::sync::Semaphore>>,
}

/// Keep the DB base and overlays atomic while async finalization advances the confirmed head.
pub(super) struct ReexecParentStateSnapshot {
    pub(super) confirmed_base_block_n: Option<u64>,
    pub(super) parent_overlays: Vec<mc_exec::ReexecParentOverlay>,
}

impl BlockProductionTask {
    pub(super) fn buffer_approved_external_content(
        &mut self,
        block_n: u64,
        source: CanonicalBlockSource,
        header: PreconfirmedHeader,
        executed_rows: Vec<PreconfirmedExecutedTransaction>,
    ) {
        let row_count = executed_rows.len();
        self.approved_external_content.insert(block_n, ApprovedExternalContent { source, header, executed_rows });
        tracing::debug!(
            block_n,
            canonical_source = ?source,
            row_count,
            buffered_blocks = self.approved_external_content.len(),
            "approved_external_content_buffered"
        );
    }

    pub(super) fn try_publish_current_external_shell(&mut self) -> anyhow::Result<()> {
        let Some(block_n) = self.backend.chain_head_state().external_preconfirmed_tip else {
            return Ok(());
        };
        let Some(content) = self.approved_external_content.remove(&block_n) else {
            return Ok(());
        };

        let row_count = content.executed_rows.len();
        let source = content.source;
        let executed_rows = content.executed_rows.clone();
        if let Err(err) =
            self.backend.write_access().fill_external_preconfirmed_shell(block_n, content.header.clone(), executed_rows)
        {
            self.approved_external_content.insert(block_n, content);
            return Err(err).with_context(|| format!("Filling external preconfirmed shell for block #{block_n}"));
        }

        tracing::debug!(
            block_n,
            canonical_source = ?source,
            row_count,
            buffered_blocks_remaining = self.approved_external_content.len(),
            "external_preconfirmed_shell_filled"
        );
        Ok(())
    }
    pub(super) fn maybe_start_canonicalization_task(&mut self) {
        if self.canonicalization_task.is_some() {
            return;
        }

        let Some(input) = self.pending_canonicalizations.pop_front() else {
            return;
        };

        // C-018: Compute parent_overlays at task start, not enqueue time.
        // This ensures overlays reflect the current canonical/runahead state,
        // not a stale snapshot from when EndBlock was first queued.
        let block_n = input.state.block_number;
        let confirmed_base = input.state.backend.latest_confirmed_block_n();
        let reexec_parent_state =
            Self::capture_reexec_parent_state(&self.diffs_since_snapshot, confirmed_base, block_n);
        tracing::debug!(
            block_n,
            confirmed_base_block_n = ?confirmed_base,
            overlay_count = reexec_parent_state.parent_overlays.len(),
            "canonicalization_overlays_recomputed"
        );

        let dispatcher = if input.state.execution_snapshot.execution_mode == ExecutionMode::Mixed {
            Some(self.reexec_dispatcher.take().unwrap_or_else(|| start_reexec_dispatcher(new_epoch_token())))
        } else {
            None
        };
        let reexec_epoch = self.reexec_epoch;
        let metrics = Arc::clone(&self.metrics);
        let no_charge_fee = self.no_charge_fee;
        let ignore_fee_token_mismatch = self.routing_cfg.runtime_options.ignore_fee_token_mismatch;
        let ignored_storage_mismatch_canonical_source =
            self.routing_cfg.runtime_options.ignored_storage_mismatch_canonical_source;
        let internal_capacity = self.close_queue_capacity;
        #[cfg(test)]
        let force_comparator_error = self.force_comparator_error;
        #[cfg(test)]
        let comparator_reexec_tx_limit = self.comparator_reexec_tx_limit;
        #[cfg(test)]
        let optimistic_pipeline_notifications = self.optimistic_pipeline_notifications.clone();
        #[cfg(test)]
        let comparator_gate = self.comparator_gates.remove(&block_n);

        let context = CanonicalizationTaskContext {
            reexec_parent_state,
            dispatcher,
            reexec_epoch,
            metrics,
            no_charge_fee,
            ignore_fee_token_mismatch,
            ignored_storage_mismatch_canonical_source,
            internal_capacity,
            #[cfg(test)]
            force_comparator_error,
            #[cfg(test)]
            comparator_reexec_tx_limit,
            #[cfg(test)]
            optimistic_pipeline_notifications,
            #[cfg(test)]
            comparator_gate,
        };

        self.canonicalization_task =
            Some(tokio::spawn(async move { Self::run_canonicalization_task(input, context).await }));
    }
    pub(super) async fn handle_canonicalization_result(
        &mut self,
        result: CanonicalizationTaskResult,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        if let Some(dispatcher) = result.dispatcher {
            self.reexec_dispatcher = Some(dispatcher);
        }

        let block_n = result.state.block_number;
        let mut state = Some(result.state);
        let canonical = match result.canonical_result {
            Ok(canonical) => canonical,
            Err(e) => {
                let mut state = state.take().expect("canonicalization state must exist on error");
                let confirmed_base = state.backend.latest_confirmed_block_n();
                let overlay_count = self.diffs_since_snapshot.len();
                tracing::debug!(
                    block_n,
                    confirmed_base_block_n = ?confirmed_base,
                    overlay_count,
                    error = format!("{e:#}"),
                    "comparator_pipeline_error"
                );
                tracing::error!(block_n, "canonical output unavailable — entering fail-safe BlockifierOnly: {e:#}");

                state.speculative_executed_txs.clear();
                {
                    let backend_clone = state.backend.clone();
                    if let Err(persist_err) = global_spawn_rayon_task(move || {
                        backend_clone
                            .write_access()
                            .flush_preconfirmed_content_to_db()
                            .context("Flushing speculative txs to DB for crash recovery")
                    })
                    .await
                    {
                        tracing::warn!(
                            block_n,
                            "Failed to flush speculative txs to DB for crash recovery: {persist_err:#}"
                        );
                    }
                }

                let recovered_descendant_rows = self.recover_descendant_pending_canonicalizations(block_n)?;

                match self.backend.rewind_internal_preconfirmed_to(block_n) {
                    Ok(n_discarded) if n_discarded > 0 => {
                        tracing::info!(block_n, n_discarded, "internal_preconfirmed_descendants_discarded");
                    }
                    Err(rewind_err) => {
                        tracing::warn!(block_n, "Failed to rewind internal preconfirmed frontier: {rewind_err:#}");
                    }
                    _ => {}
                }

                let preconfirmed_view = self
                    .backend
                    .block_view_on_preconfirmed(block_n)
                    .with_context(|| format!("No pre-confirmed block #{block_n} after pipeline fallback rewind"))?;
                let current_rows: Vec<_> =
                    preconfirmed_view.borrow_content().executed_transactions().cloned().collect();
                let carry_rows = self
                    .prepare_tainted_rebuild_carry(
                        FallbackReason::ComparatorPipelineError,
                        block_n,
                        "comparator_error",
                        BatchToExecute::default(),
                        recovered_descendant_rows,
                    )
                    .await?;
                self.install_tainted_rebuild_session(
                    block_n,
                    preconfirmed_view.block().header.clone(),
                    current_rows,
                    carry_rows,
                )?;

                anyhow::bail!(
                    "Canonical output unavailable for block {block_n}: {e:#}. \
                     Block remains preconfirmed for crash recovery."
                );
            }
        };

        tracing::debug!(
            block_n,
            canonical_source = ?canonical.canonical.source,
            "block_canonicalized"
        );

        let preconfirmed_view = state
            .as_ref()
            .expect("canonicalization state must exist")
            .backend
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("No pre-confirmed block #{block_n}"))?;

        let had_speculative =
            !state.as_ref().expect("canonicalization state must exist").speculative_executed_txs.is_empty();
        let is_stop_path = matches!(canonical.canonical.source, CanonicalBlockSource::BlockifierReexec);
        let source_ordered_speculative = if had_speculative {
            Self::speculative_rows_in_original_order(
                &state.as_ref().expect("canonicalization state must exist").speculative_executed_txs,
                &state.as_ref().expect("canonicalization state must exist").original_tx_hashes,
            )?
        } else {
            Vec::new()
        };
        let bre_rows = canonical
            .canonical
            .bre_per_tx
            .map(|per_tx| Self::build_bre_preconfirmed_prefix_rows(&source_ordered_speculative, per_tx))
            .transpose()
            .context("Building BRE-backed canonical rows")?;
        let mut canonical_rows = Some(match canonical.canonical.source {
            CanonicalBlockSource::ExecutionBox => {
                if had_speculative {
                    // Mixed mode: canonical rows come from speculative buffer.
                    state.as_ref().expect("canonicalization state must exist").speculative_executed_txs.clone()
                } else {
                    // C-024: BlockifierOnly mode — speculative_executed_txs is empty because
                    // BlockifierOnly writes directly to preconfirmed DB. Read canonical rows
                    // from the preconfirmed view (which is authoritative here since there's
                    // no async runahead in BlockifierOnly mode).
                    preconfirmed_view.borrow_content().executed_transactions().cloned().collect()
                }
            }
            CanonicalBlockSource::BlockifierReexec => bre_rows.clone().unwrap_or_else(|| {
                state.as_ref().expect("canonicalization state must exist").speculative_executed_txs.clone()
            }),
        });
        let mut pending_stop_fallback_handoff = None;
        let canonical_bouncer_weights = canonical.canonical.bouncer_weights;
        let canonical_state_diff = canonical.canonical.state_diff;

        let fallback = match canonical.stop_reason {
            Some(crate::comparator::StopReason::OutputMismatch { .. }) => {
                Some((FallbackReason::OutputMismatch, "output_mismatch"))
            }
            Some(crate::comparator::StopReason::StateDiffMismatch { .. }) => {
                Some((FallbackReason::StateDiffMismatch, "state_diff_mismatch"))
            }
            Some(crate::comparator::StopReason::ExecutionResourcesOverLimit { .. }) => {
                Some((FallbackReason::ExecResourcesOverLimit, "exec_resources_over_limit"))
            }
            None if is_stop_path => {
                Some((FallbackReason::BlockifierCanonicalSubstitution, "blockifier_canonical_substitution"))
            }
            None => None,
        };
        if let Some((fallback_reason, metric_reason)) = fallback {
            let included_prefix_len = canonical_rows.as_ref().expect("canonical rows must exist").len();
            let current_block_suffix =
                Self::collect_current_block_suffix_replay_txs(&source_ordered_speculative, included_prefix_len);
            let current_block_suffix_count = current_block_suffix.len();
            let omitted_transaction_hashes =
                current_block_suffix.txs.iter().map(|tx| format!("{:#x}", tx.tx_hash().to_felt())).collect::<Vec<_>>();
            tracing::warn!(
                target: "RUST_EXEC",
                block_n,
                execution_box_transaction_count = source_ordered_speculative.len(),
                blockifier_canonical_prefix_count = included_prefix_len,
                replay_suffix_count = current_block_suffix_count,
                omitted_transaction_hashes = ?omitted_transaction_hashes,
                fallback_reason = ?fallback_reason,
                "Comparator rejected the ExecutionBox block; preserving the Blockifier prefix and replaying the omitted suffix"
            );
            let recovered_descendant_rows = self.recover_descendant_pending_canonicalizations(block_n)?;
            pending_stop_fallback_handoff = Some(self.begin_stop_fallback_handoff(
                block_n,
                state.take().expect("strict stop must take ownership of canonicalization state"),
                canonical_bouncer_weights,
                canonical_state_diff.clone(),
                canonical_rows.take().expect("strict stop must take ownership of canonical rows"),
                preconfirmed_view.block().header.clone(),
                bre_rows.clone(),
                had_speculative,
                fallback_reason,
                metric_reason,
                current_block_suffix,
                recovered_descendant_rows,
            )?);
        }

        // C-017: On stop path, rewind internal preconfirmed to block X BEFORE
        // replacing content. Async canonicalization may have allowed internal
        // runtime to advance to X+N while comparator for X was in flight.
        // Correct order: discard descendants -> rewind to X -> replace X content.
        if is_stop_path {
            let internal_tip_before = self.backend.chain_head_state().internal_preconfirmed_tip;
            tracing::info!(
                block_n,
                internal_tip_before = ?internal_tip_before,
                "stop_path_rewind_check"
            );
            let n_discarded = self
                .backend
                .rewind_internal_preconfirmed_to(block_n)
                .with_context(|| format!("Failed to rewind internal preconfirmed frontier to block {block_n}"))?;
            if n_discarded > 0 {
                tracing::info!(block_n, n_discarded, "internal_preconfirmed_descendants_discarded");
            }
            // Validate post-rewind: internal runtime must be exactly block X.
            let post_rewind_tip = self.backend.chain_head_state().internal_preconfirmed_tip;
            anyhow::ensure!(
                post_rewind_tip == Some(block_n),
                "Post-rewind internal preconfirmed tip mismatch: expected Some({block_n}), got {post_rewind_tip:?}"
            );
        }

        if let Some(pending_handoff) = pending_stop_fallback_handoff {
            self.pending_stop_fallback_handoff = Some(pending_handoff);
            return Ok(());
        }

        let mut state = state.expect("non-stop canonicalization must retain state");
        let canonical_rows = canonical_rows.expect("non-stop canonicalization must retain canonical rows");

        if let Some(bre_rows) = bre_rows {
            let n_rows = bre_rows.len();
            state
                .backend
                .write_access()
                .replace_internal_preconfirmed_content_and_persist(block_n, bre_rows.clone())?;
            tracing::info!(block_n, n_rows, "internal_preconfirmed_replaced_with_bre_content_and_persisted");
        }

        // C-024: Clone canonical rows and header for the close payload before
        // moving them into the external publication buffer. The close payload
        // must carry its own copy so close never re-reads from preconfirmed
        // sources that may be stale under async runahead.
        let canonical_rows_for_close = canonical_rows.clone();
        let canonical_header_for_close = preconfirmed_view.block().header.clone();

        self.buffer_approved_external_content(
            block_n,
            canonical.canonical.source,
            preconfirmed_view.block().header.clone(),
            canonical_rows,
        );
        self.try_publish_current_external_shell()?;

        state.speculative_executed_txs.clear();
        if had_speculative {
            let backend_clone = state.backend.clone();
            let flush_context = if is_stop_path {
                "Flushing BRE-backed canonical preconfirmed content to DB"
            } else {
                "Flushing canonical preconfirmed content to DB (C-011A)"
            };
            global_spawn_rayon_task(move || {
                backend_clone.write_access().flush_preconfirmed_content_to_db().context(flush_context)
            })
            .await?;
            tracing::debug!(block_n, "canonical_preconfirmed_persisted");
        }

        self.enqueue_canonical_close_payload(
            close_queue,
            state,
            canonical_bouncer_weights,
            canonical_state_diff,
            canonical_rows_for_close,
            canonical_header_for_close,
        )
        .await
    }
    pub(super) fn build_bre_preconfirmed_rows(
        original_txs: &[PreconfirmedExecutedTransaction],
        bre_per_tx: Vec<ReexecExecutedTxArtifacts>,
    ) -> anyhow::Result<Vec<PreconfirmedExecutedTransaction>> {
        let mut bre_by_hash = std::collections::BTreeMap::new();
        for bre in bre_per_tx {
            let hash = *bre.receipt.transaction_hash();
            anyhow::ensure!(bre_by_hash.insert(hash, bre).is_none(), "Duplicate BRE transaction hash {hash:#x}");
        }
        anyhow::ensure!(
            bre_by_hash.len() == original_txs.len(),
            "BRE transaction membership mismatch: expected {} transaction(s), got {}",
            original_txs.len(),
            bre_by_hash.len()
        );

        original_txs
            .iter()
            .map(|orig| {
                let hash = *orig.transaction.receipt.transaction_hash();
                let bre = bre_by_hash.remove(&hash).with_context(|| {
                    format!("Missing BRE transaction hash {hash:#x} while rebuilding canonical rows")
                })?;
                let tx_reverted =
                    matches!(bre.receipt.execution_result(), mp_receipt::ExecutionResult::Reverted { .. });
                let mut state_diff = bre.tx_state_update;
                state_diff.declared_classes = Self::declared_classes_from_original_metadata(orig, tx_reverted)?;
                Ok(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt {
                        transaction: orig.transaction.transaction.clone(),
                        receipt: bre.receipt,
                    },
                    state_diff,
                    declared_class: orig.declared_class.clone(),
                    arrived_at: orig.arrived_at,
                    paid_fee_on_l1: orig.paid_fee_on_l1,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .and_then(|rows| {
                anyhow::ensure!(bre_by_hash.is_empty(), "BRE contains extra transaction(s) while rebuilding rows");
                Ok(rows)
            })
    }

    pub(super) fn speculative_rows_in_original_order(
        rows: &[PreconfirmedExecutedTransaction],
        original_tx_hashes: &[Felt],
    ) -> anyhow::Result<Vec<PreconfirmedExecutedTransaction>> {
        if original_tx_hashes.is_empty() {
            return Ok(rows.to_vec());
        }
        anyhow::ensure!(
            original_tx_hashes.len() == rows.len(),
            "Speculative transaction membership mismatch: {} original hash(es), {} executed row(s)",
            original_tx_hashes.len(),
            rows.len()
        );

        for (index, (row, expected_hash)) in rows.iter().zip(original_tx_hashes).enumerate() {
            let actual_hash = *row.transaction.receipt.transaction_hash();
            anyhow::ensure!(
                actual_hash == *expected_hash,
                "Speculative execution order mismatch at index {index}: expected {expected_hash:#x}, got {actual_hash:#x}"
            );
        }
        Ok(rows.to_vec())
    }

    pub(super) fn build_bre_preconfirmed_prefix_rows(
        source_ordered_txs: &[PreconfirmedExecutedTransaction],
        bre_per_tx: Vec<ReexecExecutedTxArtifacts>,
    ) -> anyhow::Result<Vec<PreconfirmedExecutedTransaction>> {
        let mut bre_by_hash = std::collections::BTreeMap::new();
        for bre in bre_per_tx {
            let hash = *bre.receipt.transaction_hash();
            anyhow::ensure!(bre_by_hash.insert(hash, bre).is_none(), "Duplicate BRE transaction hash {hash:#x}");
        }

        let mut prefix = Vec::with_capacity(bre_by_hash.len());
        for original in source_ordered_txs {
            let hash = *original.transaction.receipt.transaction_hash();
            let Some(bre) = bre_by_hash.remove(&hash) else {
                break;
            };
            prefix.push(Self::build_bre_preconfirmed_row(original, bre)?);
        }
        anyhow::ensure!(
            bre_by_hash.is_empty(),
            "BRE transaction result is not a contiguous prefix of original execution"
        );
        Ok(prefix)
    }

    fn build_bre_preconfirmed_row(
        original: &PreconfirmedExecutedTransaction,
        bre: ReexecExecutedTxArtifacts,
    ) -> anyhow::Result<PreconfirmedExecutedTransaction> {
        let tx_reverted = matches!(bre.receipt.execution_result(), mp_receipt::ExecutionResult::Reverted { .. });
        let mut state_diff = bre.tx_state_update;
        state_diff.declared_classes = Self::declared_classes_from_original_metadata(original, tx_reverted)?;
        Ok(PreconfirmedExecutedTransaction {
            transaction: TransactionWithReceipt {
                transaction: original.transaction.transaction.clone(),
                receipt: bre.receipt,
            },
            state_diff,
            declared_class: original.declared_class.clone(),
            arrived_at: original.arrived_at,
            paid_fee_on_l1: original.paid_fee_on_l1,
        })
    }

    /// C-023: When BRE canonicalizes only a prefix of the current speculative block X,
    pub(super) fn build_parent_overlays(
        diffs_since_snapshot: &[(u64, StateDiff)],
        confirmed_base_block_n: Option<u64>,
        target_block_n: u64,
    ) -> Vec<mc_exec::ReexecParentOverlay> {
        let first_needed = confirmed_base_block_n.map(|c| c + 1).unwrap_or(0);
        diffs_since_snapshot
            .iter()
            .filter(|(bn, _)| *bn >= first_needed && *bn < target_block_n)
            .map(|(bn, sd)| mc_exec::ReexecParentOverlay {
                block_n: *bn,
                state_diff: Self::state_diff_to_state_maps(sd),
                classes: Default::default(), // V1: classes not needed for re-exec overlay reads
            })
            .collect()
    }

    pub(super) fn capture_reexec_parent_state(
        diffs_since_snapshot: &[(u64, StateDiff)],
        confirmed_base_block_n: Option<u64>,
        target_block_n: u64,
    ) -> ReexecParentStateSnapshot {
        ReexecParentStateSnapshot {
            confirmed_base_block_n,
            parent_overlays: Self::build_parent_overlays(diffs_since_snapshot, confirmed_base_block_n, target_block_n),
        }
    }

    /// Convert `mp_state_update::StateDiff` to blockifier `StateMaps`.
    ///
    /// This is the minimal conversion needed for `LayeredStateAdapter` overlay lookups
    /// (storage, nonces, class_hashes, compiled_class_hashes).
    pub(super) fn state_diff_to_state_maps(sd: &StateDiff) -> blockifier::state::cached_state::StateMaps {
        use starknet_api::core::{ClassHash, CompiledClassHash, ContractAddress, Nonce};
        use starknet_api::state::StorageKey;

        let mut maps = blockifier::state::cached_state::StateMaps::default();

        // Storage diffs
        for contract_diff in &sd.storage_diffs {
            let addr: ContractAddress = contract_diff.address.try_into().expect("valid contract address");
            for entry in &contract_diff.storage_entries {
                let key: StorageKey = entry.key.try_into().expect("valid storage key");
                maps.storage.insert((addr, key), entry.value);
            }
        }

        // Nonces
        for nonce_update in &sd.nonces {
            let addr: ContractAddress = nonce_update.contract_address.try_into().expect("valid contract address");
            let nonce = Nonce(nonce_update.nonce);
            maps.nonces.insert(addr, nonce);
        }

        // Deployed contracts: address → class_hash
        for deployed in &sd.deployed_contracts {
            let addr: ContractAddress = deployed.address.try_into().expect("valid contract address");
            let class_hash = ClassHash(deployed.class_hash);
            maps.class_hashes.insert(addr, class_hash);
        }

        // Replaced classes: address → class_hash
        for replaced in &sd.replaced_classes {
            let addr: ContractAddress = replaced.contract_address.try_into().expect("valid contract address");
            let class_hash = ClassHash(replaced.class_hash);
            maps.class_hashes.insert(addr, class_hash);
        }

        // Declared classes: class_hash → compiled_class_hash
        for declared in &sd.declared_classes {
            let class_hash = ClassHash(declared.class_hash);
            let compiled = CompiledClassHash(declared.compiled_class_hash);
            maps.compiled_class_hashes.insert(class_hash, compiled);
        }

        // Migrated compiled classes
        for migrated in &sd.migrated_compiled_classes {
            let class_hash = ClassHash(migrated.class_hash);
            let compiled = CompiledClassHash(migrated.compiled_class_hash);
            maps.compiled_class_hashes.insert(class_hash, compiled);
        }

        maps
    }
    async fn run_canonicalization_task(
        input: PendingCanonicalizationInput,
        context: CanonicalizationTaskContext,
    ) -> anyhow::Result<CanonicalizationTaskResult> {
        let CanonicalizationTaskContext {
            reexec_parent_state,
            mut dispatcher,
            reexec_epoch,
            metrics,
            no_charge_fee,
            ignore_fee_token_mismatch,
            ignored_storage_mismatch_canonical_source,
            internal_capacity,
            #[cfg(test)]
            force_comparator_error,
            #[cfg(test)]
            comparator_reexec_tx_limit,
            #[cfg(test)]
            optimistic_pipeline_notifications,
            #[cfg(test)]
            comparator_gate,
        } = context;
        let PendingCanonicalizationInput { state, block_exec_summary } = input;
        let block_n = state.block_number;
        let execution_mode = state.execution_snapshot.execution_mode;

        #[cfg(test)]
        if execution_mode == ExecutionMode::Mixed {
            if let Some(sender) = optimistic_pipeline_notifications.as_ref() {
                let _ = sender.send(OptimisticPipelineNotification::ComparatorStarted { block_n });
            }
            if let Some(gate) = comparator_gate {
                gate.acquire().await.context("Test comparator gate closed")?.forget();
            }
        }

        let preconfirmed_view = state
            .backend
            .block_view_on_preconfirmed(block_n)
            .with_context(|| format!("No pre-confirmed block #{block_n}"))?;

        let old_declared_contracts = if execution_mode == ExecutionMode::Mixed {
            state.get_old_declared_contracts_from_speculative()
        } else {
            preconfirmed_view.get_old_declared_contracts()
        };

        let migration_v2_hashes: std::collections::HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();
        let sd_x1 = mp_state_update::StateDiff::from_blockifier(
            block_exec_summary.state_diff.clone(),
            &migration_v2_hashes,
            &state.deployed_contracts,
            old_declared_contracts,
        );

        let canonical_result = if execution_mode == ExecutionMode::Mixed {
            let dispatcher_handle = dispatcher.as_mut().expect("mixed mode requires reexec dispatcher");
            let result = Self::run_comparator_for_block_task(
                block_n,
                &state,
                &preconfirmed_view,
                &sd_x1,
                &block_exec_summary,
                dispatcher_handle,
                reexec_epoch,
                metrics,
                no_charge_fee,
                reexec_parent_state,
                ignore_fee_token_mismatch,
                ignored_storage_mismatch_canonical_source,
                internal_capacity,
                #[cfg(test)]
                force_comparator_error,
                #[cfg(test)]
                comparator_reexec_tx_limit,
            )
            .await;

            #[cfg(test)]
            if let Some(sender) = optimistic_pipeline_notifications.as_ref() {
                let _ = sender.send(OptimisticPipelineNotification::ComparatorFinished { block_n });
            }
            result
        } else {
            Ok(CanonicalizationTaskCanonical {
                canonical: CanonicalizedBlockOutput {
                    source: CanonicalBlockSource::ExecutionBox,
                    state_diff: sd_x1,
                    bouncer_weights: block_exec_summary.bouncer_weights,
                    bre_per_tx: None,
                },
                stop_reason: None,
            })
        };

        Ok(CanonicalizationTaskResult { state, canonical_result, dispatcher })
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_comparator_for_block_task(
        block_n: u64,
        state: &CurrentBlockState,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        sd_x1: &StateDiff,
        summary: &BlockExecutionSummary,
        dispatcher: &mut ReexecDispatcherHandle,
        reexec_epoch: u64,
        metrics: Arc<BlockProductionMetrics>,
        no_charge_fee: bool,
        reexec_parent_state: ReexecParentStateSnapshot,
        ignore_fee_token_mismatch: bool,
        ignored_storage_mismatch_canonical_source: RustExecCanonicalSource,
        internal_capacity: usize,
        #[cfg(test)] force_comparator_error: bool,
        #[cfg(test)] comparator_reexec_tx_limit: Option<usize>,
    ) -> anyhow::Result<CanonicalizationTaskCanonical> {
        use std::time::UNIX_EPOCH;

        #[cfg(test)]
        if force_comparator_error {
            anyhow::bail!("test failpoint: forced comparator pipeline error");
        }

        let header = &preconfirmed_view.block().header;
        let exec_ctx = crate::util::BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + std::time::Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        let parent_state_view = preconfirmed_view.state_view_on_parent();
        let source_ordered_speculative =
            Self::speculative_rows_in_original_order(&state.speculative_executed_txs, &state.original_tx_hashes)?;
        let txs: Vec<_> = source_ordered_speculative
            .iter()
            .map(|tx| Self::prepare_preconfirmed_tx_for_reexecution(tx, &parent_state_view, no_charge_fee))
            .collect::<anyhow::Result<Vec<_>>>()
            .context("Converting speculative transactions for re-execution")?;

        let ReexecParentStateSnapshot { confirmed_base_block_n: confirmed_base, parent_overlays } = reexec_parent_state;
        let tx_count = txs.len();
        let overlay_count = parent_overlays.len();

        let window = InternalPreconfirmedWindowSnapshot::from_backend(&state.backend, internal_capacity);
        tracing::info!(
            target: "RUST_EXEC",
            block_n,
            confirmed_base_block_n = ?confirmed_base,
            overlay_count,
            tx_count,
            confirmed_tip = ?window.confirmed_tip,
            external_preconfirmed_tip = ?window.external_preconfirmed_tip,
            internal_preconfirmed_tip = ?window.internal_preconfirmed_tip,
            internal_depth = window.depth,
            internal_capacity = window.capacity,
            "🔍 Comparator started for block #{} ({} transaction(s), internal {}/{})",
            block_n,
            tx_count,
            window.depth,
            window.capacity,
        );

        let req = ReexecRequest {
            epoch: reexec_epoch,
            block_n,
            backend: state.backend.clone(),
            exec_ctx,
            txs,
            deployed_contracts_set: state.deployed_contracts.clone(),
            old_declared_contracts: state.get_old_declared_contracts_from_speculative(),
            confirmed_base_block_n: confirmed_base,
            parent_overlays,
            #[cfg(test)]
            test_max_txs: comparator_reexec_tx_limit,
        };

        let reexec_start = Instant::now();
        dispatcher.req_tx.send(req).await.context("Failed to send re-execution request")?;

        let outcome = dispatcher.res_rx.recv().await.context("Re-execution dispatcher closed unexpectedly")?;
        let outcome = outcome.context("Re-execution worker failed")?;
        let reexec_duration_ms = reexec_start.elapsed().as_secs_f64() * 1000.0;

        let reexec_result = match outcome {
            ReexecWorkerOutcome::Completed(r) => {
                tracing::debug!(
                    block_n,
                    outcome = "completed",
                    duration_ms = format!("{reexec_duration_ms:.1}"),
                    "comparator_reexec_finished"
                );
                if tx_count > 0 {
                    tracing::debug!(
                        "Re-executed and compared {} transaction(s) with Blockifier for block {} - {:.3?}",
                        tx_count,
                        block_n,
                        reexec_start.elapsed(),
                    );
                }
                r
            }
            ReexecWorkerOutcome::Cancelled { epoch, block_n: bn } => {
                tracing::warn!(
                    target: "RUST_EXEC",
                    block_n = bn,
                    epoch,
                    outcome = "cancelled",
                    duration_ms = format!("{reexec_duration_ms:.1}"),
                    "comparator_reexec_finished"
                );
                anyhow::bail!("BRE cancelled for block {bn} (epoch={epoch}) — canonical output unavailable");
            }
        };

        if reexec_result.epoch != reexec_epoch {
            tracing::debug!(
                block_n,
                result_epoch = reexec_result.epoch,
                current_epoch = reexec_epoch,
                "dropping stale re-execution result"
            );
            anyhow::bail!(
                "BRE stale for block {block_n} (result_epoch={}, current_epoch={}) — canonical output unavailable",
                reexec_result.epoch,
                reexec_epoch
            );
        }

        let block_limit = state.backend.chain_config().bouncer_config.block_max_capacity;
        let compare_start = Instant::now();
        let allowed_fee_balance_keys = Self::comparator_allowed_fee_balance_keys(
            state.backend.chain_config(),
            ignore_fee_token_mismatch,
            &state.speculative_executed_txs,
            &reexec_result.per_tx,
            header.sequencer_address,
        );
        let sd_comparison = compare_state_diff_with_allowed_fee_balance_keys(
            sd_x1,
            &reexec_result.state_diff,
            &allowed_fee_balance_keys,
        );
        let er_comparison =
            compare_execution_resources(&summary.bouncer_weights, &reexec_result.exec_resources, &block_limit);
        let comparison_config = TransactionOutputComparisonConfig {
            fee_token_addresses: std::collections::BTreeSet::from([
                state.backend.chain_config().parent_fee_token_address.to_felt(),
                state.backend.chain_config().native_fee_token_address.to_felt(),
            ]),
            fee_transfer_selector: starknet_core::utils::starknet_keccak(b"Transfer"),
            sequencer_address: header.sequencer_address,
            protocol_version: header.protocol_version,
            chain_id: state.backend.chain_config().chain_id.to_felt(),
        };
        let mut comparison_report = compare_transaction_outputs(
            &state.speculative_executed_txs,
            &reexec_result.per_tx,
            &state.original_tx_hashes,
            sd_x1,
            &reexec_result.state_diff,
            matches!(sd_comparison, crate::comparator::StateDiffComparison::Mismatch { .. }),
            &comparison_config,
        );
        match &sd_comparison {
            crate::comparator::StateDiffComparison::Mismatch { summary } => {
                comparison_report.push(FieldMismatch {
                    category: MismatchCategory::StateUpdate,
                    policy: MismatchPolicy::Strict,
                    transaction_hash: None,
                    transaction_index: None,
                    field_path: "state_diff.aggregate".into(),
                    execution_box_value: format!("hash={:#x}; {summary}", sd_x1.compute_hash()),
                    blockifier_value: format!("hash={:#x}; {summary}", reexec_result.state_diff.compute_hash()),
                });
            }
            crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { mismatches } => {
                for mismatch_value in mismatches {
                    comparison_report.push(FieldMismatch {
                        category: MismatchCategory::StateUpdate,
                        policy: MismatchPolicy::Allowed,
                        transaction_hash: None,
                        transaction_index: None,
                        field_path: format!(
                            "state_diff.storage[{:#x}][{:#x}]",
                            mismatch_value.contract_address, mismatch_value.storage_key
                        ),
                        execution_box_value: format!("{:#x}", mismatch_value.execution_box_value),
                        blockifier_value: format!("{:#x}", mismatch_value.blockifier_value),
                    });
                }
            }
            crate::comparator::StateDiffComparison::Match => {}
        }
        match &er_comparison {
            ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { .. } => {
                comparison_report.push(FieldMismatch {
                    category: MismatchCategory::Resource,
                    policy: MismatchPolicy::Warning,
                    transaction_hash: None,
                    transaction_index: None,
                    field_path: "block.bouncer_weights".into(),
                    execution_box_value: format!("{:?}", summary.bouncer_weights),
                    blockifier_value: format!("{:?}", reexec_result.exec_resources),
                });
            }
            ExecutionResourceComparison::Ok if summary.bouncer_weights != reexec_result.exec_resources => {
                comparison_report.push(FieldMismatch {
                    category: MismatchCategory::Resource,
                    policy: MismatchPolicy::Diagnostic,
                    transaction_hash: None,
                    transaction_index: None,
                    field_path: "block.bouncer_weights".into(),
                    execution_box_value: format!("{:?}", summary.bouncer_weights),
                    blockifier_value: format!("{:?}", reexec_result.exec_resources),
                });
            }
            _ => {}
        }
        let decision = decide_with_report(&comparison_report, &sd_comparison, &er_comparison);
        if matches!(&er_comparison, ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. }) {
            comparison_report.push(FieldMismatch {
                category: MismatchCategory::Resource,
                policy: MismatchPolicy::Strict,
                transaction_hash: None,
                transaction_index: None,
                field_path: "block.bouncer_weights_vs_limit".into(),
                execution_box_value: format!("{:?}", summary.bouncer_weights),
                blockifier_value: format!("limit={block_limit:?}; reexec={:?}", reexec_result.exec_resources),
            });
        }
        let compare_elapsed = compare_start.elapsed().as_secs_f64();

        metrics.comparator_blocks_compared_total.add(1, &[]);
        metrics.comparator_compare_duration_seconds.record(compare_elapsed, &[]);

        match &sd_comparison {
            crate::comparator::StateDiffComparison::Mismatch { .. } => {
                metrics.comparator_state_diff_mismatch_total.add(1, &[]);
                Self::log_state_diff_mismatch_details(block_n, sd_x1, &reexec_result.state_diff, &sd_comparison);
            }
            crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { mismatches } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    allowed_fee_balance_value_mismatches = mismatches.len(),
                    "comparator_state_diff_allowed_fee_balance_mismatch"
                );
            }
            crate::comparator::StateDiffComparison::Match => {}
        }
        match &er_comparison {
            ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { .. } => {
                metrics.comparator_execbox_resources_gt_reexec_total.add(1, &[]);
                tracing::debug!(
                    block_n,
                    er_x1 = ?summary.bouncer_weights,
                    er_x2 = ?reexec_result.exec_resources,
                    block_limit = ?block_limit,
                    "comparator_execution_resources_full_dump"
                );
            }
            ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. } => {
                metrics.comparator_execbox_resources_gt_block_limit_total.add(1, &[]);
                tracing::error!(
                    target: "RUST_EXEC",
                    block_n,
                    er_x1 = ?summary.bouncer_weights,
                    er_x2 = ?reexec_result.exec_resources,
                    block_limit = ?block_limit,
                    "comparator_execution_resources_full_dump"
                );
            }
            ExecutionResourceComparison::Ok => {}
        }

        let decision_label = match &decision {
            ComparatorDecision::Accept => "accept",
            ComparatorDecision::AcceptWithWarn { .. } => "accept_with_warn",
            ComparatorDecision::StopExecutionBox { .. } => "stop_execution_box",
        };
        metrics.comparator_decisions_total.add(1, &[KeyValue::new("decision", decision_label)]);
        for ((category, policy), count) in comparison_report.mismatch_counts() {
            metrics
                .comparator_mismatches_total
                .add(count, &[KeyValue::new("category", category.as_str()), KeyValue::new("policy", policy.as_str())]);
        }
        let allowed_fee_difference_count = comparison_report.allowed_mismatches.len() as u64;
        metrics.comparator_allowed_fee_differences_total.add(allowed_fee_difference_count, &[]);
        if comparison_report.has_strict_mismatch() {
            tracing::warn!(
                target: "RUST_EXEC",
                "execution_box_state_diff_json={}",
                serde_json::to_string(sd_x1).unwrap_or_else(|error| format!("{{\"serialization_error\":{error:?}}}"))
            );
            tracing::warn!(
                target: "RUST_EXEC",
                "blockifier_recomputation_state_diff_json={}",
                serde_json::to_string(&reexec_result.state_diff)
                    .unwrap_or_else(|error| format!("{{\"serialization_error\":{error:?}}}"))
            );
            let execution_box_per_tx = state
                .speculative_executed_txs
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "transaction_hash": row.transaction.receipt.transaction_hash(),
                        "state_diff": &row.state_diff,
                    })
                })
                .collect::<Vec<_>>();
            let blockifier_per_tx = reexec_result
                .per_tx
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "transaction_hash": row.receipt.transaction_hash(),
                        "state_diff": &row.tx_state_update,
                    })
                })
                .collect::<Vec<_>>();
            tracing::warn!(
                target: "RUST_EXEC",
                "execution_box_per_transaction_state_diffs_json={}",
                serde_json::to_string(&execution_box_per_tx)
                    .unwrap_or_else(|error| format!("{{\"serialization_error\":{error:?}}}"))
            );
            tracing::warn!(
                target: "RUST_EXEC",
                "blockifier_recomputation_per_transaction_state_diffs_json={}",
                serde_json::to_string(&blockifier_per_tx)
                    .unwrap_or_else(|error| format!("{{\"serialization_error\":{error:?}}}"))
            );
            let categories = comparison_report
                .iter_mismatches()
                .map(|mismatch| mismatch.category.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            tracing::warn!(
                target: "RUST_EXEC",
                block_n,
                decision = decision_label,
                strict_mismatches = comparison_report.strict_mismatches.len(),
                allowed_mismatches = comparison_report.allowed_mismatches.len(),
                resource_warnings = comparison_report.resource_warnings.len(),
                diagnostics = comparison_report.diagnostics.len(),
                affected_transactions = comparison_report.affected_transaction_hashes.len(),
                execution_box_transactions = comparison_report.execution_box_transaction_count,
                blockifier_transactions = comparison_report.blockifier_transaction_count,
                canonical_transactions = comparison_report.canonical_transaction_count,
                paired_transactions = comparison_report.paired_transaction_count,
                categories = ?categories,
                "comparator_output_mismatch_summary"
            );
            for mismatch in comparison_report.iter_mismatches().take(20) {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    transaction_index = ?mismatch.transaction_index,
                    transaction_hash = ?mismatch.transaction_hash.map(|hash| format!("{hash:#x}")),
                    category = mismatch.category.as_str(),
                    policy = mismatch.policy.as_str(),
                    field_path = %mismatch.field_path,
                    execution_box = %mismatch.execution_box_value,
                    blockifier = %mismatch.blockifier_value,
                    decision = decision_label,
                    "comparator_output_mismatch_detail"
                );
            }
        }

        let allowed_fee_mismatch = !comparison_report.allowed_mismatches.is_empty();
        let use_blockifier_on_allowed_fee_mismatch = allowed_fee_mismatch
            && ignored_storage_mismatch_canonical_source == RustExecCanonicalSource::BlockifierReexec;
        let sd_match = matches!(
            &sd_comparison,
            crate::comparator::StateDiffComparison::Match
                | crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { .. }
        );
        let er_match = matches!(er_comparison, ExecutionResourceComparison::Ok);

        let (canonical, stop_reason) = match &decision {
            ComparatorDecision::Accept => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "accept",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    overlay_count,
                    "comparator_passed"
                );
                if use_blockifier_on_allowed_fee_mismatch {
                    tracing::warn!(
                        target: "RUST_EXEC",
                        block_n,
                        "comparator_allowed_fee_mismatch_using_blockifier_canonical_output"
                    );
                    (
                        CanonicalizedBlockOutput {
                            source: CanonicalBlockSource::BlockifierReexec,
                            state_diff: reexec_result.state_diff,
                            bouncer_weights: reexec_result.exec_resources,
                            bre_per_tx: Some(reexec_result.per_tx),
                        },
                        None,
                    )
                } else {
                    (
                        CanonicalizedBlockOutput {
                            source: CanonicalBlockSource::ExecutionBox,
                            state_diff: sd_x1.clone(),
                            bouncer_weights: summary.bouncer_weights,
                            bre_per_tx: None,
                        },
                        None,
                    )
                }
            }
            ComparatorDecision::AcceptWithWarn { .. } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "accept_with_warn",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    overlay_count,
                    "comparator_passed"
                );
                if use_blockifier_on_allowed_fee_mismatch {
                    tracing::warn!(
                        target: "RUST_EXEC",
                        block_n,
                        "comparator_allowed_fee_mismatch_using_blockifier_canonical_output"
                    );
                    (
                        CanonicalizedBlockOutput {
                            source: CanonicalBlockSource::BlockifierReexec,
                            state_diff: reexec_result.state_diff,
                            bouncer_weights: reexec_result.exec_resources,
                            bre_per_tx: Some(reexec_result.per_tx),
                        },
                        None,
                    )
                } else {
                    (
                        CanonicalizedBlockOutput {
                            source: CanonicalBlockSource::ExecutionBox,
                            state_diff: sd_x1.clone(),
                            bouncer_weights: summary.bouncer_weights,
                            bre_per_tx: None,
                        },
                        None,
                    )
                }
            }
            ComparatorDecision::StopExecutionBox { reason } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "stop",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    reason = %reason,
                    "comparator_failed"
                );
                (
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::BlockifierReexec,
                        state_diff: reexec_result.state_diff,
                        bouncer_weights: reexec_result.exec_resources,
                        bre_per_tx: Some(reexec_result.per_tx),
                    },
                    Some(reason.clone()),
                )
            }
        };

        tracing::debug!(
            block_n,
            canonical_source = ?canonical.source,
            "canonicalization_source_selected"
        );

        let comparator_duration_ms = reexec_start.elapsed().as_secs_f64() * 1000.0;
        let state_diff_result = match &sd_comparison {
            crate::comparator::StateDiffComparison::Match => "matched",
            crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { .. } => {
                "matched with allowed fee-token balance differences"
            }
            crate::comparator::StateDiffComparison::Mismatch { .. } => "mismatched",
        };
        let resource_result = match &er_comparison {
            ExecutionResourceComparison::Ok => "within comparator limits",
            ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { .. } => {
                "ExecutionBox greater than Blockifier but within block limit"
            }
            ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. } => "over block limit",
        };
        let canonical_source = match canonical.source {
            CanonicalBlockSource::ExecutionBox => "ExecutionBox",
            CanonicalBlockSource::BlockifierReexec => "Blockifier",
        };
        let allowed_fee_balance_value_mismatches = match &sd_comparison {
            crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { mismatches } => mismatches.len(),
            _ => 0,
        };

        match &decision {
            ComparatorDecision::Accept => tracing::info!(
                target: "RUST_EXEC",
                block_n,
                tx_count,
                overlay_count,
                duration_ms = comparator_duration_ms,
                state_diff_result,
                allowed_fee_balance_value_mismatches,
                resource_result,
                canonical_source,
                "✅ Comparator passed for block #{} in {:.3?} (state diff: {}; resources: {}; canonical: {})",
                block_n,
                reexec_start.elapsed(),
                state_diff_result,
                resource_result,
                canonical_source,
            ),
            ComparatorDecision::AcceptWithWarn { resource_deltas } => tracing::warn!(
                target: "RUST_EXEC",
                block_n,
                tx_count,
                overlay_count,
                duration_ms = comparator_duration_ms,
                state_diff_result,
                allowed_fee_balance_value_mismatches,
                resource_result,
                canonical_source,
                resource_deltas = %resource_deltas,
                "✅ Comparator passed for block #{} in {:.3?} (state diff: {}; resources: {}; canonical: {})",
                block_n,
                reexec_start.elapsed(),
                state_diff_result,
                resource_result,
                canonical_source,
            ),
            ComparatorDecision::StopExecutionBox { reason } => tracing::error!(
                target: "RUST_EXEC",
                block_n,
                tx_count,
                overlay_count,
                duration_ms = comparator_duration_ms,
                state_diff_result,
                allowed_fee_balance_value_mismatches,
                resource_result,
                canonical_source,
                reason = %reason,
                "❌ Comparator failed for block #{} in {:.3?} ({}; canonical: {}; switching to Blockifier-only)",
                block_n,
                reexec_start.elapsed(),
                reason,
                canonical_source,
            ),
        }

        Ok(CanonicalizationTaskCanonical { canonical, stop_reason })
    }

    /// Run comparator for block `block_n` (Mixed mode only, T-054 / C-009A / C-010A / C-010B).
    ///
    /// Builds a `ReexecRequest` from the in-memory speculative artifacts (C-010A),
    /// dispatches it to the async re-execution worker, and on completion runs the
    /// comparator decision layer.
    ///
    /// Returns `CanonicalizedBlockOutput` indicating which execution source is canonical
    /// for block X. The caller must use the returned state_diff and bouncer_weights for
    /// close pipeline input.
    ///
    /// - On `Accept` or `AcceptWithWarn`: canonical from EB (outputs-1).
    /// - On `StopExecutionBox`: canonical from BRE (outputs-2), enters fallback.
    /// - On `Cancelled` or stale epoch: returns `Err` (C-010B — BRE unavailable).
    #[allow(dead_code)]
    pub(super) async fn run_comparator_for_block(
        &mut self,
        block_n: u64,
        state: &CurrentBlockState,
        preconfirmed_view: &MadaraPreconfirmedBlockView,
        sd_x1: &StateDiff,
        summary: &BlockExecutionSummary,
    ) -> anyhow::Result<CanonicalizedBlockOutput> {
        use std::time::UNIX_EPOCH;

        // Test-only failpoint: simulate comparator pipeline failure.
        #[cfg(test)]
        if self.force_comparator_error {
            anyhow::bail!("test failpoint: forced comparator pipeline error");
        }

        // Lazily create the dispatcher on first Mixed-mode block.
        if self.reexec_dispatcher.is_none() {
            let token = new_epoch_token();
            self.reexec_dispatcher = Some(start_reexec_dispatcher(token));
        }

        // Build the request before borrowing dispatcher (avoids mutable + immutable borrow conflict).
        let header = &preconfirmed_view.block().header;
        let exec_ctx = crate::util::BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + std::time::Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        let parent_state_view = preconfirmed_view.state_view_on_parent();
        let no_charge_fee = self.no_charge_fee;
        // C-010A: Use in-memory speculative buffer for transaction list (not preconfirmed view,
        // which has no executed txs in mixed mode since writes were deferred).
        let source_ordered_speculative =
            Self::speculative_rows_in_original_order(&state.speculative_executed_txs, &state.original_tx_hashes)?;
        let txs: Vec<_> = source_ordered_speculative
            .iter()
            .map(|tx| Self::prepare_preconfirmed_tx_for_reexecution(tx, &parent_state_view, no_charge_fee))
            .collect::<anyhow::Result<Vec<_>>>()
            .context("Converting speculative transactions for re-execution")?;

        // Build synthetic parent overlays for runahead re-execution (C-007B).
        let confirmed_base = state.backend.latest_confirmed_block_n();
        let parent_overlays = Self::build_parent_overlays(&self.diffs_since_snapshot, confirmed_base, block_n);
        let tx_count = txs.len();
        let overlay_count = parent_overlays.len();

        tracing::debug!(
            block_n,
            confirmed_base_block_n = ?confirmed_base,
            overlay_count,
            tx_count,
            "comparator_started"
        );

        let req = ReexecRequest {
            epoch: self.reexec_epoch,
            block_n,
            backend: state.backend.clone(),
            exec_ctx,
            txs,
            deployed_contracts_set: state.deployed_contracts.clone(),
            // C-010A: Compute from speculative buffer (preconfirmed has no executed txs).
            old_declared_contracts: state.get_old_declared_contracts_from_speculative(),
            confirmed_base_block_n: confirmed_base,
            parent_overlays,
            #[cfg(test)]
            test_max_txs: self.comparator_reexec_tx_limit,
        };

        // Send to dispatcher and await result (inline in V1, no pipeline buffering yet).
        let reexec_start = Instant::now();
        let dispatcher = self.reexec_dispatcher.as_mut().expect("dispatcher just created");
        dispatcher.req_tx.send(req).await.context("Failed to send re-execution request")?;

        let outcome = dispatcher.res_rx.recv().await.context("Re-execution dispatcher closed unexpectedly")?;
        let outcome = outcome.context("Re-execution worker failed")?;
        let reexec_duration_ms = reexec_start.elapsed().as_secs_f64() * 1000.0;

        let reexec_result = match outcome {
            ReexecWorkerOutcome::Completed(r) => {
                tracing::debug!(
                    block_n,
                    outcome = "completed",
                    duration_ms = format!("{reexec_duration_ms:.1}"),
                    "comparator_reexec_finished"
                );
                if tx_count > 0 {
                    tracing::debug!(
                        "🔁 Re-executed and compared {} transaction(s) with Blockifier for block {} - {:.3?}",
                        tx_count,
                        block_n,
                        reexec_start.elapsed(),
                    );
                }
                r
            }
            ReexecWorkerOutcome::Cancelled { epoch, block_n: bn } => {
                tracing::warn!(
                    target: "RUST_EXEC",
                    block_n = bn,
                    epoch,
                    outcome = "cancelled",
                    duration_ms = format!("{reexec_duration_ms:.1}"),
                    "comparator_reexec_finished"
                );
                // C-010B: BRE unavailable — do not silently canonicalize from speculative EB.
                anyhow::bail!("BRE cancelled for block {bn} (epoch={epoch}) — canonical output unavailable");
            }
        };

        // C-010B: Drop stale results from a previous epoch — BRE unavailable.
        if reexec_result.epoch != self.reexec_epoch {
            tracing::debug!(
                block_n,
                result_epoch = reexec_result.epoch,
                current_epoch = self.reexec_epoch,
                "dropping stale re-execution result"
            );
            anyhow::bail!(
                "BRE stale for block {block_n} (result_epoch={}, current_epoch={}) — canonical output unavailable",
                reexec_result.epoch,
                self.reexec_epoch
            );
        }

        // Run the pure comparison functions (timed for comparator_compare_duration_seconds).
        let block_limit = state.backend.chain_config().bouncer_config.block_max_capacity;
        let compare_start = Instant::now();
        let allowed_fee_balance_keys = Self::comparator_allowed_fee_balance_keys(
            state.backend.chain_config(),
            self.routing_cfg.runtime_options.ignore_fee_token_mismatch,
            &state.speculative_executed_txs,
            &reexec_result.per_tx,
            header.sequencer_address,
        );
        let sd_comparison = compare_state_diff_with_allowed_fee_balance_keys(
            sd_x1,
            &reexec_result.state_diff,
            &allowed_fee_balance_keys,
        );
        let er_comparison =
            compare_execution_resources(&summary.bouncer_weights, &reexec_result.exec_resources, &block_limit);
        let decision = decide(&sd_comparison, &er_comparison);
        let compare_elapsed = compare_start.elapsed().as_secs_f64();

        // Emit metrics.
        self.metrics.comparator_blocks_compared_total.add(1, &[]);
        self.metrics.comparator_compare_duration_seconds.record(compare_elapsed, &[]);

        match &sd_comparison {
            crate::comparator::StateDiffComparison::Mismatch { .. } => {
                self.metrics.comparator_state_diff_mismatch_total.add(1, &[]);
                Self::log_state_diff_mismatch_details(block_n, sd_x1, &reexec_result.state_diff, &sd_comparison);
            }
            crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { mismatches } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    allowed_fee_balance_value_mismatches = mismatches.len(),
                    "comparator_state_diff_allowed_fee_balance_mismatch"
                );
            }
            crate::comparator::StateDiffComparison::Match => {}
        }
        match &er_comparison {
            ExecutionResourceComparison::WarnExecutionBoxGreaterThanReexec { .. } => {
                self.metrics.comparator_execbox_resources_gt_reexec_total.add(1, &[]);
                tracing::debug!(
                    block_n,
                    er_x1 = ?summary.bouncer_weights,
                    er_x2 = ?reexec_result.exec_resources,
                    block_limit = ?block_limit,
                    "comparator_execution_resources_full_dump"
                );
            }
            ExecutionResourceComparison::FatalExecutionBoxGreaterThanBlockLimit { .. } => {
                self.metrics.comparator_execbox_resources_gt_block_limit_total.add(1, &[]);
                tracing::error!(
                    target: "RUST_EXEC",
                    block_n,
                    er_x1 = ?summary.bouncer_weights,
                    er_x2 = ?reexec_result.exec_resources,
                    block_limit = ?block_limit,
                    "comparator_execution_resources_full_dump"
                );
            }
            ExecutionResourceComparison::Ok => {}
        }

        let allowed_fee_balance_mismatch =
            matches!(&sd_comparison, crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { .. });
        let use_blockifier_on_allowed_fee_balance_mismatch = allowed_fee_balance_mismatch
            && self.routing_cfg.runtime_options.ignored_storage_mismatch_canonical_source
                == RustExecCanonicalSource::BlockifierReexec;
        let sd_match = matches!(
            &sd_comparison,
            crate::comparator::StateDiffComparison::Match
                | crate::comparator::StateDiffComparison::AllowedFeeBalanceMismatch { .. }
        );
        let er_match = matches!(er_comparison, ExecutionResourceComparison::Ok);

        // Select canonical source based on comparator decision (C-009A).
        let canonical = match &decision {
            ComparatorDecision::Accept => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "accept",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    overlay_count,
                    "comparator_passed"
                );
                if use_blockifier_on_allowed_fee_balance_mismatch {
                    tracing::warn!(
                        target: "RUST_EXEC",
                        block_n,
                        "comparator_allowed_fee_balance_mismatch_using_blockifier_canonical_output"
                    );
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::BlockifierReexec,
                        state_diff: reexec_result.state_diff,
                        bouncer_weights: reexec_result.exec_resources,
                        bre_per_tx: Some(reexec_result.per_tx),
                    }
                } else {
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::ExecutionBox,
                        state_diff: sd_x1.clone(),
                        bouncer_weights: summary.bouncer_weights,
                        bre_per_tx: None,
                    }
                }
            }
            ComparatorDecision::AcceptWithWarn { .. } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "accept_with_warn",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    overlay_count,
                    "comparator_passed"
                );
                if use_blockifier_on_allowed_fee_balance_mismatch {
                    tracing::warn!(
                        target: "RUST_EXEC",
                        block_n,
                        "comparator_allowed_fee_balance_mismatch_using_blockifier_canonical_output"
                    );
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::BlockifierReexec,
                        state_diff: reexec_result.state_diff,
                        bouncer_weights: reexec_result.exec_resources,
                        bre_per_tx: Some(reexec_result.per_tx),
                    }
                } else {
                    CanonicalizedBlockOutput {
                        source: CanonicalBlockSource::ExecutionBox,
                        state_diff: sd_x1.clone(),
                        bouncer_weights: summary.bouncer_weights,
                        bre_per_tx: None,
                    }
                }
            }
            ComparatorDecision::StopExecutionBox { reason } => {
                tracing::debug!(
                    target: "RUST_EXEC",
                    block_n,
                    decision = "stop",
                    state_diff_match = sd_match,
                    resources_match = er_match,
                    reason = %reason,
                    "comparator_failed"
                );
                // Runtime fallback / tainted-rebuild handoff now lives in `handle_canonicalization_result`.
                // This legacy helper only returns the canonical source decision.
                // C-013: Pass BRE per-tx artifacts for BRE-backed external promotion.
                // Empty per_tx (from incomplete collection) triggers EB-backed fallback.
                CanonicalizedBlockOutput {
                    source: CanonicalBlockSource::BlockifierReexec,
                    state_diff: reexec_result.state_diff,
                    bouncer_weights: reexec_result.exec_resources,
                    bre_per_tx: Some(reexec_result.per_tx),
                }
            }
        };

        tracing::debug!(
            block_n,
            canonical_source = ?canonical.source,
            "canonicalization_source_selected"
        );

        Ok(canonical)
    }

    fn log_state_diff_mismatch_details(
        block_n: u64,
        rust_exec_diff: &StateDiff,
        blockifier_diff: &StateDiff,
        comparison: &crate::comparator::StateDiffComparison,
    ) {
        let crate::comparator::StateDiffComparison::Mismatch { summary } = comparison else {
            return;
        };

        let (storage_mismatch_count, storage_mismatch_preview) =
            Self::storage_diff_mismatch_preview_json(rust_exec_diff, blockifier_diff, 16);
        let summary = summary.to_string();
        let storage_mismatch_preview =
            serde_json::to_string(&storage_mismatch_preview).unwrap_or_else(|err| format!("json_error:{err}"));
        let dump_path = Self::write_state_diff_mismatch_dump(block_n, &summary, rust_exec_diff, blockifier_diff, 256)
            .unwrap_or_else(|| "unavailable".to_owned());
        tracing::warn!(
            target: "RUST_EXEC",
            block_n,
            summary = %summary,
            rust_exec_storage_contracts = rust_exec_diff.storage_diffs.len(),
            blockifier_storage_contracts = blockifier_diff.storage_diffs.len(),
            rust_exec_storage_entries = Self::storage_entry_count(rust_exec_diff),
            blockifier_storage_entries = Self::storage_entry_count(blockifier_diff),
            storage_mismatch_count,
            storage_mismatch_preview = %storage_mismatch_preview,
            dump_path = %dump_path,
            "comparator_state_diff_mismatch_details block_n={} summary={} rust_exec_storage_contracts={} blockifier_storage_contracts={} rust_exec_storage_entries={} blockifier_storage_entries={} storage_mismatch_count={} storage_mismatch_preview={} dump_path={}",
            block_n,
            summary,
            rust_exec_diff.storage_diffs.len(),
            blockifier_diff.storage_diffs.len(),
            Self::storage_entry_count(rust_exec_diff),
            Self::storage_entry_count(blockifier_diff),
            storage_mismatch_count,
            storage_mismatch_preview,
            dump_path
        );
    }

    fn storage_entry_count(diff: &StateDiff) -> usize {
        diff.storage_diffs.iter().map(|item| item.storage_entries.len()).sum()
    }

    pub(super) fn storage_diff_mismatch_preview_json(
        rust_exec_diff: &StateDiff,
        blockifier_diff: &StateDiff,
        preview_limit: usize,
    ) -> (usize, Vec<serde_json::Value>) {
        let rust_exec_storage = Self::flatten_storage_diff(rust_exec_diff);
        let blockifier_storage = Self::flatten_storage_diff(blockifier_diff);
        let mut keys = std::collections::BTreeSet::new();
        keys.extend(rust_exec_storage.keys().copied());
        keys.extend(blockifier_storage.keys().copied());

        let mut mismatch_count = 0;
        let mut preview = Vec::new();
        for (address, key) in keys {
            let rust_exec_value = rust_exec_storage.get(&(address, key));
            let blockifier_value = blockifier_storage.get(&(address, key));
            if rust_exec_value == blockifier_value {
                continue;
            }

            mismatch_count += 1;
            if preview.len() < preview_limit {
                let kind = match (rust_exec_value, blockifier_value) {
                    (None, Some(_)) => "missing_in_rust_exec",
                    (Some(_), None) => "extra_in_rust_exec",
                    (Some(_), Some(_)) => "value_mismatch",
                    (None, None) => unreachable!("mismatch key must exist in at least one diff"),
                };
                preview.push(serde_json::json!({
                    "kind": kind,
                    "contract_address": Self::felt_hex(address),
                    "storage_key": Self::felt_hex(key),
                    "rust_exec_value": rust_exec_value.map(|value| Self::felt_hex(*value)),
                    "blockifier_value": blockifier_value.map(|value| Self::felt_hex(*value)),
                }));
            }
        }

        if mismatch_count > preview.len() {
            preview.push(serde_json::json!({
                "truncated": true,
                "remaining": mismatch_count - preview.len(),
            }));
        }

        (mismatch_count, preview)
    }

    fn flatten_storage_diff(diff: &StateDiff) -> BTreeMap<(Felt, Felt), Felt> {
        diff.storage_diffs
            .iter()
            .flat_map(|item| item.storage_entries.iter().map(move |entry| ((item.address, entry.key), entry.value)))
            .collect()
    }

    fn write_state_diff_mismatch_dump(
        block_n: u64,
        summary: &str,
        rust_exec_diff: &StateDiff,
        blockifier_diff: &StateDiff,
        full_mismatch_limit: usize,
    ) -> Option<String> {
        let (storage_mismatch_count, storage_mismatch_preview) =
            Self::storage_diff_mismatch_preview_json(rust_exec_diff, blockifier_diff, full_mismatch_limit);
        let path = format!("/tmp/madara-comparator-state-diff-mismatch-block-{block_n}.json");
        let payload = serde_json::json!({
            "block_n": block_n,
            "summary": summary,
            "storage_mismatch_count": storage_mismatch_count,
            "storage_mismatch_preview": storage_mismatch_preview,
            "rust_exec": {
                "storage_contracts": rust_exec_diff.storage_diffs.len(),
                "storage_entries": Self::storage_entry_count(rust_exec_diff),
                "storage_diffs": Self::storage_diff_json_entries(rust_exec_diff),
                "full_state_diff": rust_exec_diff,
            },
            "blockifier": {
                "storage_contracts": blockifier_diff.storage_diffs.len(),
                "storage_entries": Self::storage_entry_count(blockifier_diff),
                "storage_diffs": Self::storage_diff_json_entries(blockifier_diff),
                "full_state_diff": blockifier_diff,
            },
        });

        let bytes = match serde_json::to_vec_pretty(&payload) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    target: "RUST_EXEC",
                    block_n,
                    error = %err,
                    "comparator_state_diff_mismatch_dump_serialize_failed"
                );
                return None;
            }
        };

        if let Err(err) = std::fs::write(&path, bytes) {
            tracing::warn!(
                target: "RUST_EXEC",
                block_n,
                path = %path,
                error = %err,
                "comparator_state_diff_mismatch_dump_write_failed"
            );
            return None;
        }

        Some(path)
    }

    fn storage_diff_json_entries(diff: &StateDiff) -> Vec<serde_json::Value> {
        Self::flatten_storage_diff(diff)
            .into_iter()
            .map(|((address, key), value)| {
                serde_json::json!({
                    "contract_address": Self::felt_hex(address),
                    "storage_key": Self::felt_hex(key),
                    "value": Self::felt_hex(value),
                })
            })
            .collect()
    }

    fn felt_hex(value: Felt) -> String {
        format!("0x{value:x}")
    }
}
