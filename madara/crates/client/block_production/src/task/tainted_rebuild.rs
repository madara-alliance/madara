use super::*;

impl BlockProductionTask {
    pub(super) fn tainted_rebuild_sources_from_carry_rows(
        rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) -> Vec<TaintedRebuildSourceTx> {
        rows.into_iter()
            .map(|row| TaintedRebuildSourceTx {
                validated: row.tx,
                source_block_n: row.source_block_n,
                force_charge_fee: Some(row.effective_charge_fee),
            })
            .collect()
    }

    pub(super) fn tainted_rebuild_sources_from_preconfirmed_rows(
        block_n: u64,
        rows: Vec<PreconfirmedExecutedTransaction>,
        no_charge_fee: bool,
    ) -> Vec<TaintedRebuildSourceTx> {
        rows.into_iter()
            .map(|row| TaintedRebuildSourceTx {
                validated: row.to_validated(),
                source_block_n: Some(block_n),
                force_charge_fee: Some(!no_charge_fee),
            })
            .collect()
    }

    pub(super) fn tainted_rebuild_carry_rows_from_sources(
        source_txs: Vec<TaintedRebuildSourceTx>,
    ) -> Vec<mc_db::StoredTaintedRebuildCarryRow> {
        source_txs
            .into_iter()
            .enumerate()
            .map(|(seq_no, source)| {
                let effective_charge_fee = source.effective_charge_fee();
                let TaintedRebuildSourceTx { validated, source_block_n, .. } = source;
                mc_db::StoredTaintedRebuildCarryRow {
                    seq_no: seq_no as u64,
                    declared_class: validated.declared_class.clone(),
                    arrived_at: validated.arrived_at,
                    tx: validated,
                    source_block_n,
                    effective_charge_fee,
                }
            })
            .collect()
    }

    pub(super) fn tainted_rebuild_carry_rows_from_sources_for_next_block(
        next_block_n: u64,
        source_txs: Vec<TaintedRebuildSourceTx>,
    ) -> Vec<mc_db::StoredTaintedRebuildCarryRow> {
        Self::tainted_rebuild_carry_rows_from_sources(
            source_txs
                .into_iter()
                .map(|mut source| {
                    source.source_block_n = Some(next_block_n);
                    source
                })
                .collect(),
        )
    }

    pub(super) fn tainted_rebuild_carry_rows_from_batch(
        batch: BatchToExecute,
        source_block_n: Option<u64>,
    ) -> Vec<mc_db::StoredTaintedRebuildCarryRow> {
        batch
            .into_iter()
            .enumerate()
            .map(|(seq_no, (tx, info))| {
                let validated = Self::validated_tx_from_blockifier(tx, info);
                mc_db::StoredTaintedRebuildCarryRow {
                    seq_no: seq_no as u64,
                    declared_class: validated.declared_class.clone(),
                    arrived_at: validated.arrived_at,
                    effective_charge_fee: validated.charge_fee,
                    tx: validated,
                    source_block_n,
                }
            })
            .collect()
    }

    pub(super) fn tainted_rebuild_carry_rows_from_executor_carry(
        carry: Vec<util::TaintedRebuildCarryTx>,
    ) -> Vec<mc_db::StoredTaintedRebuildCarryRow> {
        carry
            .into_iter()
            .enumerate()
            .map(|(seq_no, carry_tx)| {
                let validated = Self::validated_tx_from_blockifier(carry_tx.tx, carry_tx.additional_info);
                mc_db::StoredTaintedRebuildCarryRow {
                    seq_no: seq_no as u64,
                    declared_class: validated.declared_class.clone(),
                    arrived_at: validated.arrived_at,
                    effective_charge_fee: validated.charge_fee,
                    tx: validated,
                    source_block_n: carry_tx.source_block_n,
                }
            })
            .collect()
    }

    pub(super) fn merge_dedup_tainted_rebuild_carry_rows(
        primary: Vec<mc_db::StoredTaintedRebuildCarryRow>,
        secondary: Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) -> Vec<mc_db::StoredTaintedRebuildCarryRow> {
        let mut seen = HashSet::new();
        primary
            .into_iter()
            .chain(secondary)
            .filter(|row| seen.insert(row.tx.hash))
            .enumerate()
            .map(|(seq_no, mut row)| {
                row.seq_no = seq_no as u64;
                row
            })
            .collect()
    }

    pub(super) fn merge_dedup_tainted_rebuild_sources(
        primary: Vec<TaintedRebuildSourceTx>,
        secondary: Vec<TaintedRebuildSourceTx>,
    ) -> Vec<TaintedRebuildSourceTx> {
        let mut seen = HashSet::new();
        primary.into_iter().chain(secondary).filter(|source| seen.insert(source.validated.hash)).collect()
    }

    pub(super) fn partition_tainted_rebuild_carry_rows(
        rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
        next_block_n: u64,
    ) -> (
        Vec<mc_db::StoredTaintedRebuildCarryRow>,
        Vec<mc_db::StoredTaintedRebuildCarryRow>,
        Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) {
        let mut immediate = Vec::new();
        let mut same_block = Vec::new();
        let mut future = Vec::new();

        for row in rows {
            match row.source_block_n {
                Some(source_block_n) if source_block_n > next_block_n => future.push(row),
                Some(source_block_n) if source_block_n == next_block_n => same_block.push(row),
                Some(_) | None => immediate.push(row),
            }
        }

        (immediate, same_block, future)
    }

    pub(super) fn tainted_rebuild_sources_for_saved_block(
        next_block_n: u64,
        carry_rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
        saved_rows: Vec<PreconfirmedExecutedTransaction>,
        no_charge_fee: bool,
    ) -> (Vec<TaintedRebuildSourceTx>, Vec<mc_db::StoredTaintedRebuildCarryRow>) {
        let (immediate_carry, same_block_carry, future_carry) =
            Self::partition_tainted_rebuild_carry_rows(carry_rows, next_block_n);
        let mut source_txs = Self::tainted_rebuild_sources_from_carry_rows(immediate_carry);
        source_txs.extend(Self::merge_dedup_tainted_rebuild_sources(
            Self::tainted_rebuild_sources_from_carry_rows(same_block_carry),
            Self::tainted_rebuild_sources_from_preconfirmed_rows(next_block_n, saved_rows, no_charge_fee),
        ));
        (source_txs, future_carry)
    }

    pub(super) fn tainted_rebuild_sources_for_overflow_block(
        next_block_n: u64,
        carry_rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) -> (Vec<TaintedRebuildSourceTx>, Vec<mc_db::StoredTaintedRebuildCarryRow>) {
        let (immediate_carry, same_block_carry, future_carry) =
            Self::partition_tainted_rebuild_carry_rows(carry_rows, next_block_n);
        let mut source_txs = Self::tainted_rebuild_sources_from_carry_rows(immediate_carry);
        source_txs.extend(Self::tainted_rebuild_sources_from_carry_rows(same_block_carry));
        (source_txs, future_carry)
    }

    pub(super) fn tainted_rebuild_live_session_after_step(
        session: &mc_db::StoredTaintedRebuildSession,
        next_session: Option<mc_db::StoredTaintedRebuildSession>,
    ) -> Option<mc_db::StoredTaintedRebuildSession> {
        next_session.or_else(|| {
            Some(if session.next_block_n <= session.tail_block_n {
                mc_db::StoredTaintedRebuildSession {
                    next_block_n: session.next_block_n.saturating_add(1),
                    ..session.clone()
                }
            } else {
                session.clone()
            })
        })
    }

    pub(super) fn build_bre_preconfirmed_rows_from_sources(
        original_txs: &[TaintedRebuildSourceTx],
        bre_per_tx: Vec<ReexecExecutedTxArtifacts>,
    ) -> anyhow::Result<Vec<PreconfirmedExecutedTransaction>> {
        original_txs
            .iter()
            .zip(bre_per_tx)
            .map(|(orig, bre)| {
                anyhow::ensure!(
                    bre.receipt.transaction_hash() == &orig.validated.hash,
                    "Transaction hash mismatch while rebuilding canonical row: expected {:#x}, got {:#x}",
                    orig.validated.hash,
                    bre.receipt.transaction_hash()
                );
                let tx_reverted =
                    matches!(bre.receipt.execution_result(), mp_receipt::ExecutionResult::Reverted { .. });
                let mut state_diff = bre.tx_state_update;
                state_diff.declared_classes = Self::declared_classes_from_validated_metadata(
                    &orig.validated.transaction,
                    orig.validated.declared_class.as_ref(),
                    tx_reverted,
                )?;
                Ok(PreconfirmedExecutedTransaction {
                    transaction: TransactionWithReceipt {
                        transaction: orig.validated.transaction.clone(),
                        receipt: bre.receipt,
                    },
                    state_diff,
                    declared_class: orig.validated.declared_class.clone(),
                    arrived_at: orig.validated.arrived_at,
                    paid_fee_on_l1: orig.validated.paid_fee_on_l1,
                })
            })
            .collect()
    }

    pub(super) fn old_declared_contracts_from_rows(rows: &[PreconfirmedExecutedTransaction]) -> Vec<Felt> {
        let mut old_declared_contracts = Vec::new();
        for tx in rows {
            for (&class_hash, &compiled_class_hash) in &tx.state_diff.declared_classes {
                if matches!(compiled_class_hash, DeclaredClassCompiledClass::Legacy) {
                    old_declared_contracts.push(class_hash);
                }
            }
        }
        old_declared_contracts.sort();
        old_declared_contracts
    }

    pub(super) fn deployed_contracts_set_from_rows(rows: &[PreconfirmedExecutedTransaction]) -> HashSet<Felt> {
        let mut deployed_contracts = HashSet::new();
        for tx in rows {
            for (&address, update) in &tx.state_diff.contract_class_hashes {
                if matches!(update, ClassUpdateItem::DeployedContract(_)) {
                    deployed_contracts.insert(address);
                }
            }
        }
        deployed_contracts
    }

    pub(super) fn tainted_rebuild_active(&self) -> bool {
        self.tainted_rebuild_handoff_pending
            || self.tainted_rebuild_session.is_some()
            || self.tainted_rebuild_task.is_some()
    }

    pub(super) fn publish_tainted_rebuild_gate(&self) {
        let _ = self.tainted_rebuild_active_tx.send(self.tainted_rebuild_active());
    }

    pub(super) async fn run_tainted_rebuild_step_task(
        backend: Arc<MadaraBackend>,
        session: mc_db::StoredTaintedRebuildSession,
        fallback_no_charge_fee: bool,
    ) -> anyhow::Result<TaintedRebuildStepResult> {
        let rebuild_started_at = Instant::now();
        let carry_rows = backend.get_tainted_rebuild_carry_rows().context("Loading tainted rebuild carry rows")?;
        let (saved_chain_config, saved_no_charge_fee) =
            Self::load_saved_runtime_exec_config(&backend, fallback_no_charge_fee)?;

        let (header, mut source_txs, future_carry_rows) = if session.next_block_n <= session.tail_block_n {
            let (header, saved_rows) = backend
                .db
                .get_preconfirmed_block_data(session.next_block_n)?
                .with_context(|| format!("Missing persisted tainted descendant block #{}", session.next_block_n))?;
            let (source_txs, future_carry_rows) = Self::tainted_rebuild_sources_for_saved_block(
                session.next_block_n,
                carry_rows,
                saved_rows,
                saved_no_charge_fee,
            );
            (header, source_txs, future_carry_rows)
        } else {
            let (source_txs, future_carry_rows) =
                Self::tainted_rebuild_sources_for_overflow_block(session.next_block_n, carry_rows);
            anyhow::ensure!(
                !source_txs.is_empty(),
                "Tainted rebuild overflow block #{} requested with empty carry",
                session.next_block_n
            );
            (Self::overflow_rebuild_header(&backend, session.next_block_n)?, source_txs, future_carry_rows)
        };

        let parent_state_view = backend.view_on_latest_confirmed();
        let blockifier_txs: Vec<_> = source_txs
            .iter()
            .map(|source| {
                Self::prepare_validated_tx_for_reexecution(
                    &source.validated,
                    &parent_state_view,
                    source.force_charge_fee,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .context("Converting tainted rebuild transactions to blockifier format")?;

        anyhow::ensure!(
            !blockifier_txs.is_empty(),
            "Tainted rebuild block #{} has no input transactions to rebuild",
            header.block_number
        );

        let exec_ctx = util::BlockExecutionContext {
            block_number: header.block_number,
            sequencer_address: header.sequencer_address,
            block_timestamp: UNIX_EPOCH + Duration::from_secs(header.block_timestamp.0),
            protocol_version: header.protocol_version,
            gas_prices: header.gas_prices.clone(),
            l1_da_mode: header.l1_da_mode,
        };

        let state_adapter =
            LayeredStateAdapter::new(backend.clone()).context("Creating tainted rebuild state adaptor")?;
        let mut executor = util::create_executor_with_block_n_min_10(
            &backend,
            &exec_ctx,
            state_adapter,
            |block_n| Self::confirmed_block_min_10_hash(&backend, block_n),
            saved_chain_config.as_ref(),
        )
        .context("Creating tainted rebuild executor")?;

        let execution_results = executor.execute_txs(&blockifier_txs, None);
        let rebuilt_len = execution_results.len();
        anyhow::ensure!(
            rebuilt_len > 0,
            "Tainted rebuild block #{} could not fit any transaction into the rebuilt block",
            header.block_number
        );

        let mut per_tx_artifacts = Vec::with_capacity(rebuilt_len);
        let mut deployed_in_block = HashSet::new();
        for (i, (result, blockifier_tx)) in execution_results.into_iter().zip(blockifier_txs.iter()).enumerate() {
            match result {
                Ok((execution_info, state_maps)) => {
                    let receipt = from_blockifier_execution_info(&execution_info, blockifier_tx);
                    let tx_state_update = Self::convert_blockifier_state_maps_for_preconfirmed_row(
                        &backend,
                        state_maps,
                        &mut deployed_in_block,
                    )
                    .with_context(|| {
                        format!("Converting state maps for tainted rebuild tx {} in block #{}", i, header.block_number)
                    })?;
                    per_tx_artifacts.push(ReexecExecutedTxArtifacts { receipt, tx_state_update });
                }
                Err(err) => {
                    anyhow::bail!(
                        "Transaction {} (hash: {:#x}) failed during tainted rebuild re-execution: {err:?}",
                        i,
                        source_txs[i].validated.hash
                    );
                }
            }
        }

        let canonical_rows =
            Self::build_bre_preconfirmed_rows_from_sources(&source_txs[..rebuilt_len], per_tx_artifacts).with_context(
                || format!("Building canonical rows for tainted rebuild block #{}", header.block_number),
            )?;
        let consumed_core_contract_nonces = canonical_rows
            .iter()
            .filter_map(|tx| tx.transaction.transaction.as_l1_handler().map(|l1_tx| l1_tx.nonce))
            .collect::<HashSet<_>>();

        let block_exec_summary = executor.finalize().context("Finalizing tainted rebuild executor")?;
        let old_declared_contracts = Self::old_declared_contracts_from_rows(&canonical_rows);
        let deployed_contracts_set = Self::deployed_contracts_set_from_rows(&canonical_rows);
        let migration_v2_hashes: HashSet<Felt> = block_exec_summary
            .compiled_class_hashes_for_migration
            .iter()
            .map(|(v2_hash, _v1_hash)| v2_hash.0)
            .collect();
        let state_diff = StateDiff::from_blockifier(
            block_exec_summary.state_diff,
            &migration_v2_hashes,
            &deployed_contracts_set,
            old_declared_contracts,
        );

        let carry_rows = Self::merge_dedup_tainted_rebuild_carry_rows(
            Self::tainted_rebuild_carry_rows_from_sources_for_next_block(
                header.block_number.saturating_add(1),
                source_txs.split_off(rebuilt_len),
            ),
            future_carry_rows,
        );
        let next_session = if carry_rows.is_empty() && session.next_block_n >= session.tail_block_n {
            None
        } else {
            Some(mc_db::StoredTaintedRebuildSession {
                next_block_n: session.next_block_n.saturating_add(1),
                ..session.clone()
            })
        };
        let live_session_after_step = Self::tainted_rebuild_live_session_after_step(&session, next_session.clone());

        backend
            .stage_tainted_rebuild_preconfirmed_block(&header, &canonical_rows, next_session.as_ref(), &carry_rows)
            .context("Persisting tainted rebuild block staging state")?;
        backend
            .write_access()
            .new_preconfirmed(PreconfirmedBlock::new_with_content(header.clone(), canonical_rows.clone(), []))
            .context("Publishing tainted rebuild preconfirmed block runtime state")?;

        let reverted_count = canonical_rows
            .iter()
            .filter(|tx| {
                matches!(tx.transaction.receipt.execution_result(), mp_receipt::ExecutionResult::Reverted { .. })
            })
            .count();
        let declared_classes_count = canonical_rows.iter().filter(|tx| tx.declared_class.is_some()).count();
        let mut state =
            CurrentBlockState::with_execution_mode(backend, header.block_number, ExecutionMode::BlockifierOnly);
        state.block_start_time = rebuild_started_at;
        state.consumed_core_contract_nonces = consumed_core_contract_nonces;
        state.accumulated_stats.n_batches = 1;
        state.accumulated_stats.n_added_to_block = canonical_rows.len();
        state.accumulated_stats.n_executed = canonical_rows.len();
        state.accumulated_stats.n_reverted = reverted_count;
        state.accumulated_stats.declared_classes = declared_classes_count;

        Ok(TaintedRebuildStepResult {
            live_session_after_step,
            close_payload: TaintedRebuildClosePayload {
                state,
                canonical_bouncer_weights: block_exec_summary.bouncer_weights,
                state_diff,
                canonical_executed_rows: canonical_rows,
                canonical_header: header,
            },
        })
    }

    pub(super) fn maybe_finish_tainted_rebuild_if_drained(&mut self) -> anyhow::Result<bool> {
        let Some(session) = self.tainted_rebuild_session.clone() else {
            return Ok(false);
        };
        if session.next_block_n <= session.tail_block_n {
            return Ok(false);
        }
        if !self.backend.get_tainted_rebuild_carry_rows()?.is_empty() {
            return Ok(false);
        }

        self.backend.clear_tainted_rebuild_session().context("Clearing drained tainted rebuild session")?;
        self.backend.clear_tainted_rebuild_carry_rows().context("Clearing drained tainted rebuild carry rows")?;
        self.tainted_rebuild_session = None;
        self.tainted_rebuild_handoff_pending = false;

        if self.fallback.startup_recovery_active {
            self.fallback.on_startup_recovery_complete();
            let _ = self.execution_mode_tx.send(self.fallback.mode);
        }

        self.queue_post_close_executor_resync(None, "drained tainted rebuild")
            .context("Queueing executor resync after drained tainted rebuild")?;
        self.publish_tainted_rebuild_gate();

        Ok(true)
    }

    pub(super) fn queue_post_close_executor_resync(
        &self,
        wait_for_confirmed_block_n: Option<u64>,
        reason: &'static str,
    ) -> anyhow::Result<()> {
        match self.handle.resync_to_backend_head(wait_for_confirmed_block_n) {
            Ok(()) => Ok(()),
            // During graceful shutdown the final close completion can arrive after the
            // executor has already emitted EndFinalBlock and torn down its command loop.
            // At that point there is no speculative frontier left to realign, so the
            // post-close resync is intentionally a no-op.
            Err(executor::ExecutorCommandError::ChannelClosed) => {
                tracing::info!(
                    reason,
                    wait_for_confirmed_block_n = ?wait_for_confirmed_block_n,
                    "executor_already_stopped_skipping_post_close_resync"
                );
                Ok(())
            }
        }
    }

    pub(super) fn maybe_start_tainted_rebuild_task(&mut self) -> anyhow::Result<()> {
        if self.tainted_rebuild_task.is_some() || self.tainted_rebuild_handoff_pending {
            return Ok(());
        }
        let Some(session) = self.tainted_rebuild_session.clone() else {
            return Ok(());
        };
        if self.backend.latest_confirmed_block_n().is_none_or(|confirmed_tip| confirmed_tip < session.anchor_block_n) {
            return Ok(());
        }
        if !matches!(self.current_state, Some(TaskState::NotExecuting { .. })) {
            return Ok(());
        }
        if !self.pending_canonicalizations.is_empty()
            || self.canonicalization_task.is_some()
            || !self.pending_completions.is_empty()
        {
            return Ok(());
        }
        if self.maybe_finish_tainted_rebuild_if_drained()? {
            return Ok(());
        }

        let backend = self.backend.clone();
        let fallback_no_charge_fee = self.no_charge_fee;
        self.tainted_rebuild_task = Some(tokio::spawn(async move {
            Self::run_tainted_rebuild_step_task(backend, session, fallback_no_charge_fee).await
        }));
        self.publish_tainted_rebuild_gate();
        Ok(())
    }

    /// Enter fail-safe BlockifierOnly fallback: flip mode, broadcast via watch channel,
    /// emit stop metric, cancel in-flight re-exec workers, and increment epoch.
    ///
    /// Build Blockifier-backed `PreconfirmedExecutedTransaction` rows for stop-path promotion
    /// and startup recovery.
    ///
    /// Combines original tx metadata (payload, arrived_at, paid_fee_on_l1, declared_class)
    /// with Blockifier execution artifacts (receipt, tx state update).
    /// the remaining suffix of X must be replayed before descendants X+1..N.
    pub(super) fn collect_current_block_suffix_replay_txs(
        speculative_executed_txs: &[PreconfirmedExecutedTransaction],
        included_prefix_len: usize,
    ) -> BatchToExecute {
        let mut replay = BatchToExecute::default();
        for preconfirmed_tx in speculative_executed_txs.iter().skip(included_prefix_len) {
            let validated = preconfirmed_tx.to_validated();
            match validated.into_blockifier_for_sequencing() {
                Ok((btx, ts, declared_class)) => {
                    replay.push(btx, AdditionalTxInfo { declared_class, arrived_at: ts });
                }
                Err(e) => {
                    tracing::warn!(
                        tx_hash = format!("{:#x}", preconfirmed_tx.transaction.receipt.transaction_hash()),
                        "Failed to convert current-block suffix tx for replay: {e:#}"
                    );
                }
            }
        }
        replay
    }

    /// Drop stale descendant canonicalization queue entries while leaving persisted descendant
    /// preconfirmed buckets intact as the durable rebuild source of truth.
    pub(super) fn drop_descendant_pending_canonicalizations(&mut self, stop_block_n: u64) {
        let mut purged_blocks = Vec::new();

        let mut kept = VecDeque::new();
        for entry in self.pending_canonicalizations.drain(..) {
            if entry.state.block_number <= stop_block_n {
                kept.push_back(entry);
            } else {
                purged_blocks.push(entry.state.block_number);
            }
        }
        self.pending_canonicalizations = kept;

        let n_purged = purged_blocks.len();
        if n_purged > 0 {
            tracing::info!(
                stop_block_n,
                n_purged,
                purged_blocks = ?purged_blocks,
                "descendant_canonicalizations_purged_stale_queue_entries"
            );
        }
    }

    pub(super) fn spawn_tainted_rebuild_carry_task(
        &mut self,
        reason: FallbackReason,
        block_n: u64,
        metric_reason: &str,
        current_block_suffix: BatchToExecute,
    ) -> anyhow::Result<PendingTaintedRebuildCarry> {
        let previous_mode = self.fallback.mode;
        self.metrics.comparator_executionbox_stop_total.add(1, &[KeyValue::new("reason", metric_reason.to_string())]);

        self.fallback.enter_fallback(reason, block_n);
        let new_mode = self.fallback.mode;
        let _ = self.execution_mode_tx.send(new_mode);

        tracing::info!(
            block_n,
            reason = ?reason,
            previous_mode = ?previous_mode,
            new_mode = ?new_mode,
            current_block_suffix_txs = current_block_suffix.len(),
            "executionbox_fallback_entered"
        );

        self.execution_epoch += 1;
        let _ = self.execution_epoch_tx.send(self.execution_epoch);
        self.tainted_rebuild_handoff_pending = true;
        self.publish_tainted_rebuild_gate();

        // C-020: Sanitize main-task current_state so replay StartNewBlock(X+1) is accepted.
        // After stop on block X, any speculative descendant state (Executing or NotExecuting
        // with latest_block_n > X) is stale and must be reset.
        match &self.current_state {
            Some(TaskState::Executing(state)) if state.block_number > block_n => {
                tracing::info!(
                    block_n,
                    stale_block_number = state.block_number,
                    "failsafe_discarding_stale_executing_descendant_state"
                );
                self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(block_n) });
            }
            Some(TaskState::NotExecuting { latest_block_n: Some(n) }) if *n > block_n => {
                tracing::info!(
                    block_n,
                    stale_latest_block_n = *n,
                    "failsafe_clamping_stale_not_executing_latest_block_n"
                );
                self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(block_n) });
            }
            _ => {
                // current_state is at or below block_n, or is None — leave unchanged.
            }
        }

        // Cancel all in-flight re-exec workers for this epoch and reset.
        if let Some(d) = self.reexec_dispatcher.take() {
            d.cancel();
        }
        self.reexec_epoch += 1;

        let current_block_suffix_rows =
            Self::tainted_rebuild_carry_rows_from_batch(current_block_suffix, Some(block_n));
        let reply_rx = self
            .handle
            .request_tainted_rebuild_fallback(block_n, self.execution_epoch)
            .context("Queueing executor tainted rebuild fallback carry request")?;
        let carry_task = tokio::spawn(async move {
            let executor_carry = reply_rx
                .await
                .map_err(|_| executor::ExecutorCommandError::ChannelClosed)?
                .context("Preparing executor tainted rebuild fallback carry")?;
            Ok(Self::merge_dedup_tainted_rebuild_carry_rows(
                current_block_suffix_rows,
                Self::tainted_rebuild_carry_rows_from_executor_carry(executor_carry),
            ))
        });

        Ok(PendingTaintedRebuildCarry { carry_task })
    }

    pub(super) async fn prepare_tainted_rebuild_carry(
        &mut self,
        reason: FallbackReason,
        block_n: u64,
        metric_reason: &str,
        current_block_suffix: BatchToExecute,
    ) -> anyhow::Result<Vec<mc_db::StoredTaintedRebuildCarryRow>> {
        let pending = self.spawn_tainted_rebuild_carry_task(reason, block_n, metric_reason, current_block_suffix)?;
        self.await_tainted_rebuild_carry(pending).await.context("Awaiting tainted rebuild carry handoff")
    }

    pub(super) async fn await_tainted_rebuild_carry(
        &self,
        pending: PendingTaintedRebuildCarry,
    ) -> anyhow::Result<Vec<mc_db::StoredTaintedRebuildCarryRow>> {
        pending
            .carry_task
            .await
            .context("Tainted rebuild carry task join failed")?
            .context("Resolving tainted rebuild carry handoff")
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn begin_stop_fallback_handoff(
        &mut self,
        block_n: u64,
        state: CurrentBlockState,
        canonical_bouncer_weights: blockifier::bouncer::BouncerWeights,
        canonical_state_diff: StateDiff,
        canonical_rows: Vec<PreconfirmedExecutedTransaction>,
        canonical_header: PreconfirmedHeader,
        bre_rows: Option<Vec<PreconfirmedExecutedTransaction>>,
        had_speculative: bool,
        fallback_reason: FallbackReason,
        metric_reason: &str,
        current_block_suffix: BatchToExecute,
    ) -> anyhow::Result<PendingStopFallbackHandoff> {
        let carry =
            self.spawn_tainted_rebuild_carry_task(fallback_reason, block_n, metric_reason, current_block_suffix)?;
        Ok(PendingStopFallbackHandoff {
            block_n,
            state,
            canonical_bouncer_weights,
            canonical_state_diff,
            canonical_rows,
            canonical_header,
            replace_internal_preconfirmed_rows: bre_rows,
            had_speculative,
            carry,
        })
    }

    pub(super) async fn finish_pending_stop_fallback_handoff(
        &mut self,
        close_queue: &FinalizerHandle,
        carry_rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) -> anyhow::Result<()> {
        let pending = self.pending_stop_fallback_handoff.take().context("No pending strict stop fallback handoff")?;
        let block_n = pending.block_n;

        if let Some(rows) = pending.replace_internal_preconfirmed_rows.as_ref() {
            let n_rows = rows.len();
            self.install_tainted_rebuild_session(block_n, pending.canonical_header.clone(), rows.clone(), carry_rows)?;
            pending.state.backend.write_access().replace_internal_preconfirmed_content(block_n, rows.clone())?;
            tracing::info!(block_n, n_rows, "internal_preconfirmed_replaced_with_bre_content_and_persisted");
        } else {
            self.install_tainted_rebuild_session(
                block_n,
                pending.canonical_header.clone(),
                pending.canonical_rows.clone(),
                carry_rows,
            )?;
        }

        self.buffer_approved_external_content(
            block_n,
            CanonicalBlockSource::BlockifierReexec,
            pending.canonical_header.clone(),
            pending.canonical_rows.clone(),
        );
        self.try_publish_current_external_shell()?;

        let mut state = pending.state;
        state.speculative_executed_txs.clear();
        if pending.had_speculative {
            let backend_clone = state.backend.clone();
            global_spawn_rayon_task(move || {
                backend_clone
                    .write_access()
                    .flush_preconfirmed_content_to_db()
                    .context("Flushing BRE-backed canonical preconfirmed content to DB")
            })
            .await?;
            tracing::info!(block_n, "canonical_preconfirmed_persisted");
        }

        self.enqueue_canonical_close_payload(
            close_queue,
            state,
            pending.canonical_bouncer_weights,
            pending.canonical_state_diff,
            pending.canonical_rows,
            pending.canonical_header,
        )
        .await
    }

    #[cfg(test)]
    pub(super) async fn complete_pending_stop_fallback_handoff(
        &mut self,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let carry_task = {
            let pending =
                self.pending_stop_fallback_handoff.as_mut().context("No pending strict stop fallback handoff")?;
            mem::replace(&mut pending.carry.carry_task, tokio::spawn(async { Ok(Vec::new()) }))
        };
        let carry_rows = self.await_tainted_rebuild_carry(PendingTaintedRebuildCarry { carry_task }).await?;
        self.finish_pending_stop_fallback_handoff(close_queue, carry_rows).await
    }

    pub(super) fn install_tainted_rebuild_session(
        &mut self,
        block_n: u64,
        header: PreconfirmedHeader,
        canonical_rows: Vec<PreconfirmedExecutedTransaction>,
        carry_rows: Vec<mc_db::StoredTaintedRebuildCarryRow>,
    ) -> anyhow::Result<()> {
        let tail_block_n = self
            .backend
            .db
            .get_latest_preconfirmed_header_block_n()
            .context("Reading tainted rebuild tail block")?
            .unwrap_or(block_n);
        let next_block_n = block_n.saturating_add(1);
        let session = if carry_rows.is_empty() && tail_block_n <= block_n {
            None
        } else {
            Some(mc_db::StoredTaintedRebuildSession {
                execution_epoch: self.execution_epoch,
                anchor_block_n: block_n,
                next_block_n,
                tail_block_n,
                active: true,
            })
        };

        self.backend
            .stage_tainted_rebuild_preconfirmed_block(&header, &canonical_rows, session.as_ref(), &carry_rows)
            .context("Persisting tainted rebuild anchor state")?;

        self.tainted_rebuild_session = session;
        self.tainted_rebuild_handoff_pending = false;
        self.publish_tainted_rebuild_gate();
        self.save_current_runtime_exec_config()?;
        Ok(())
    }
}
