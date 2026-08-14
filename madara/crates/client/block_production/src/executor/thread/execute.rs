use super::*;
use crate::executor::{BatchExecutionResult, ExecutorCommand, ExecutorMessage};

impl ExecutorThread {
    pub fn run(mut self) -> anyhow::Result<()> {
        let block_time = self.backend.chain_config().block_time;
        let no_empty_blocks = self.backend.chain_config().no_empty_blocks;

        // Initial state is ExecutorState::NewBlock, we don't yet have an execution state.
        let mut state = self.initial_state().context("Creating executor initial state")?;

        // T-031: pending routed work (replaces to_exec + replay_next_block_buffer).
        // Intake fills this when empty; both phases drain from their respective branches.
        let mut pending_routed = RoutedBatchToExecute::default();

        let mut next_block_deadline = Instant::now() + block_time;
        let mut force_close = false;
        let mut block_empty = true;
        let mut l2_gas_consumed_block = 0;
        let mut execution_epoch = *self.execution_epoch_rx.borrow();
        let mut desired_execution_mode = *self.execution_mode_rx.borrow();
        let mut tainted_rebuild_parked = self.start_tainted_rebuild_parked;
        let mut runtime_replay_active = false;
        let mut replay_current_block_active = false;

        tracing::debug!("Starting executor thread.");
        self.publish_replay_status(runtime_replay_active, execution_epoch);

        // The goal here is to do the least possible between batches, as to maximize CPU usage.
        loop {
            while !tainted_rebuild_parked {
                let Ok(cmd) = self.commands.try_recv() else {
                    break;
                };
                match cmd {
                    ExecutorCommand::CloseBlock(callback) => {
                        force_close = true;
                        let _ = callback.send(Ok(()));
                    }
                    ExecutorCommand::SetDesiredExecutionMode { mode } => {
                        self.apply_desired_execution_mode(&mut desired_execution_mode, &state, mode);
                    }
                    ExecutorCommand::PrepareTaintedRebuildFallback { block_n, execution_epoch: new_epoch, reply } => {
                        desired_execution_mode = ExecutionMode::BlockifierOnly;
                        self.publish_effective_execution_mode(desired_execution_mode);
                        execution_epoch = new_epoch;
                        let carry = self.prepare_tainted_rebuild_fallback(
                            &mut state,
                            &mut pending_routed,
                            block_n,
                            execution_epoch,
                            &mut next_block_deadline,
                            &mut force_close,
                            &mut block_empty,
                            &mut l2_gas_consumed_block,
                            block_time,
                        )?;
                        let _ = reply.send(Ok(carry));
                        tainted_rebuild_parked = true;
                        runtime_replay_active = false;
                        replay_current_block_active = false;
                        self.publish_replay_status(runtime_replay_active, execution_epoch);
                        tracing::info!(
                            target: "RUST_EXEC",
                            block_n,
                            execution_epoch,
                            "Executor parked for tainted rebuild"
                        );
                    }
                    ExecutorCommand::ResumeAfterTaintedRebuild { reply, .. } => {
                        let _ = reply.send(Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(
                            "executor is not parked".to_owned(),
                        )));
                    }
                }
            }

            if tainted_rebuild_parked {
                let command = self.wait_rt.block_on(async {
                    tokio::time::timeout(std::time::Duration::from_millis(100), self.commands.recv()).await
                });
                let command = match command {
                    Ok(Some(command)) => command,
                    Ok(None) => {
                        self.finish_shutdown(state, execution_epoch);
                        return Ok(());
                    }
                    Err(_) if self.incoming_batches.is_closed() => {
                        self.finish_shutdown(state, execution_epoch);
                        return Ok(());
                    }
                    Err(_) => continue,
                };
                match command {
                    ExecutorCommand::CloseBlock(callback) => {
                        let _ = callback.send(Err(crate::executor::ExecutorCommandError::TaintedRebuildActive));
                    }
                    ExecutorCommand::SetDesiredExecutionMode { mode } => {
                        if mode != ExecutionMode::BlockifierOnly {
                            tracing::warn!(
                                target: "RUST_EXEC",
                                ?mode,
                                "Ignored mode change while executor is parked for tainted rebuild"
                            );
                        }
                    }
                    ExecutorCommand::PrepareTaintedRebuildFallback { block_n, execution_epoch: new_epoch, reply } => {
                        desired_execution_mode = ExecutionMode::BlockifierOnly;
                        self.publish_effective_execution_mode(desired_execution_mode);
                        execution_epoch = new_epoch;
                        let carry = self.prepare_tainted_rebuild_fallback(
                            &mut state,
                            &mut pending_routed,
                            block_n,
                            execution_epoch,
                            &mut next_block_deadline,
                            &mut force_close,
                            &mut block_empty,
                            &mut l2_gas_consumed_block,
                            block_time,
                        )?;
                        let _ = reply.send(Ok(carry));
                        tracing::info!(
                            target: "RUST_EXEC",
                            block_n,
                            execution_epoch,
                            "Executor remains parked for tainted rebuild"
                        );
                    }
                    ExecutorCommand::ResumeAfterTaintedRebuild {
                        expected_confirmed_head,
                        execution_epoch: resume_epoch,
                        reply,
                    } => {
                        let result = if resume_epoch != execution_epoch {
                            Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(format!(
                                "expected execution epoch {execution_epoch}, got {resume_epoch}"
                            )))
                        } else {
                            self.resume_after_tainted_rebuild(
                                &mut state,
                                &mut pending_routed,
                                &mut desired_execution_mode,
                                execution_epoch,
                                expected_confirmed_head,
                                &mut runtime_replay_active,
                                &mut replay_current_block_active,
                                &mut next_block_deadline,
                                &mut force_close,
                                &mut block_empty,
                                &mut l2_gas_consumed_block,
                                block_time,
                            )
                        };
                        if let Ok(ack) = result.as_ref() {
                            tainted_rebuild_parked = false;
                            tracing::info!(
                                target: "RUST_EXEC",
                                confirmed_head = ack.confirmed_head,
                                next_block_n = ack.next_block_n,
                                execution_epoch = ack.execution_epoch,
                                "Executor resumed after tainted rebuild"
                            );
                        }
                        let _ = reply.send(result);
                    }
                }
                continue;
            }

            // ── Intake ──────────────────────────────────────────────────────────────────
            // Fetch a new routed payload only when pending work is fully exhausted.
            // Once we have pending work, we proceed directly to execution without waiting.
            if pending_routed.is_empty() {
                let replay_boundary_active = self.replay_mode_enabled
                    && matches!(
                        &state,
                        ExecutorThreadState::Executing(s)
                            if self.replay_boundary_exists(s.exec_ctx.block_number)
                    );
                let wait_deadline = if replay_boundary_active || (block_empty && no_empty_blocks) {
                    None
                } else {
                    Some(next_block_deadline)
                };
                let current_block_n = match &state {
                    ExecutorThreadState::Executing(s) => Some(s.exec_ctx.block_number),
                    ExecutorThreadState::NewBlock(s) => Some(s.state_adaptor.block_n()),
                };
                let prefer_commands = matches!(state, ExecutorThreadState::NewBlock(_));

                let (mut taken, preserve_pending_routed) = if force_close {
                    (RoutedBatchToExecute::default(), false)
                } else {
                    match self.wait_take_tx_batch(
                        current_block_n,
                        wait_deadline,
                        /* should_wait */ true,
                        prefer_commands,
                    ) {
                        WaitTxBatchOutcome::Batch(b) => (b, false),
                        WaitTxBatchOutcome::Command(ExecutorCommand::CloseBlock(callback)) => {
                            force_close = true;
                            let _ = callback.send(Ok(()));
                            (RoutedBatchToExecute::default(), false)
                        }
                        WaitTxBatchOutcome::Command(ExecutorCommand::SetDesiredExecutionMode { mode }) => {
                            self.apply_desired_execution_mode(&mut desired_execution_mode, &state, mode);
                            continue;
                        }
                        WaitTxBatchOutcome::Command(ExecutorCommand::PrepareTaintedRebuildFallback {
                            block_n,
                            execution_epoch: new_epoch,
                            reply,
                        }) => {
                            desired_execution_mode = ExecutionMode::BlockifierOnly;
                            self.publish_effective_execution_mode(desired_execution_mode);
                            execution_epoch = new_epoch;
                            let carry = self.prepare_tainted_rebuild_fallback(
                                &mut state,
                                &mut pending_routed,
                                block_n,
                                execution_epoch,
                                &mut next_block_deadline,
                                &mut force_close,
                                &mut block_empty,
                                &mut l2_gas_consumed_block,
                                block_time,
                            )?;
                            let _ = reply.send(Ok(carry));
                            tainted_rebuild_parked = true;
                            runtime_replay_active = false;
                            replay_current_block_active = false;
                            self.publish_replay_status(runtime_replay_active, execution_epoch);
                            tracing::info!(
                                target: "RUST_EXEC",
                                block_n,
                                execution_epoch,
                                "Executor parked for tainted rebuild"
                            );
                            (RoutedBatchToExecute::default(), true)
                        }
                        WaitTxBatchOutcome::Command(ExecutorCommand::ResumeAfterTaintedRebuild { reply, .. }) => {
                            let _ =
                                reply.send(Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(
                                    "executor is not parked".to_owned(),
                                )));
                            continue;
                        }
                        // Channel closed — graceful shutdown.
                        WaitTxBatchOutcome::Exit => {
                            self.finish_shutdown(state, execution_epoch);
                            return Ok(());
                        }
                    }
                };

                if preserve_pending_routed {
                    continue;
                }

                // C-021: If a routed payload from the old speculative epoch arrives after
                // fallback, recover it into Blockifier replay instead of executing/dropping it.
                if taken.execution_epoch != execution_epoch {
                    let stale_epoch = taken.execution_epoch;
                    let mut recovered = taken.blockifier_batch;
                    recovered.extend(taken.rust_batch);
                    tracing::debug!(
                        stale_epoch,
                        current_epoch = execution_epoch,
                        recovered_txs = recovered.len(),
                        "executor_recovering_stale_routed_batch_after_fallback"
                    );
                    taken = RoutedBatchToExecute {
                        blockifier_batch: recovered,
                        rust_batch: BatchToExecute::default(),
                        original_tx_hashes: taken.original_tx_hashes,
                        block_n: taken.block_n,
                        execution_mode: ExecutionMode::BlockifierOnly,
                        execution_epoch,
                    };
                }

                // Dedup L1 handler nonces in blockifier_batch.
                // rust_batch contains only Invoke txs (by classifier design) so no dedup needed there.
                let mut deduped_blockifier = BatchToExecute::with_capacity(taken.blockifier_batch.len());
                for (tx, info) in taken.blockifier_batch {
                    if let Some(nonce) = tx.l1_handler_tx_nonce() {
                        let nonce: u64 = nonce.to_felt().try_into().context("Converting nonce from felt to u64")?;
                        if state
                            .layered_state_adapter_mut()
                            .is_l1_to_l2_message_nonce_consumed(nonce)
                            .context("Checking is l1 to l2 message nonce is already consumed")?
                            || !state.consumed_l1_to_l2_nonces().insert(nonce)
                        {
                            tracing::debug!("L1 Core Contract nonce already consumed: {nonce}");
                            continue;
                        }
                    }
                    deduped_blockifier.push(tx, info);
                }
                pending_routed = RoutedBatchToExecute {
                    blockifier_batch: deduped_blockifier,
                    rust_batch: taken.rust_batch,
                    original_tx_hashes: taken.original_tx_hashes,
                    block_n: taken.block_n,
                    execution_mode: taken.execution_mode,
                    execution_epoch: taken.execution_epoch,
                };
                pending_routed.normalize_original_tx_hashes();
                Self::rebind_routed_batch_to_block(&mut pending_routed, Self::current_executor_block_n(&state));

                if matches!(
                    self.sync_new_block_boundary_commands(
                        &mut state,
                        &mut pending_routed,
                        &mut desired_execution_mode,
                        &mut execution_epoch,
                        &mut tainted_rebuild_parked,
                        &mut runtime_replay_active,
                        &mut replay_current_block_active,
                        &mut next_block_deadline,
                        &mut force_close,
                        &mut block_empty,
                        &mut l2_gas_consumed_block,
                        block_time,
                    )?,
                    NewBlockBoundarySyncOutcome::ContinueOuterLoop
                ) {
                    continue;
                }

                let target_execution_mode = match &state {
                    ExecutorThreadState::Executing(s) => s.execution_mode,
                    ExecutorThreadState::NewBlock(_) => desired_execution_mode,
                };
                Self::normalize_routed_batch_for_execution_mode(&mut pending_routed, target_execution_mode);

                if !pending_routed.is_empty() {
                    let summary = summarize_routed_batch(&pending_routed);
                    self.metrics.executor_routed_payloads_total.add(1, &[]);
                    tracing::debug!(
                        total_txs = pending_routed.total_len(),
                        execution_mode = ?pending_routed.execution_mode,
                        execution_epoch = pending_routed.execution_epoch,
                        first_tx_hash = summary.first_hash.as_deref().unwrap_or("-"),
                        first_tx_nonce = summary.first_nonce.as_deref().unwrap_or("-"),
                        last_tx_hash = summary.last_hash.as_deref().unwrap_or("-"),
                        last_tx_nonce = summary.last_nonce.as_deref().unwrap_or("-"),
                        "executor_accepted_routed_batch"
                    );
                }
            }

            // ── Create execution state (NewBlock → Executing) ────────────────────────
            if matches!(state, ExecutorThreadState::NewBlock(_)) && !pending_routed.is_empty() {
                if matches!(
                    self.sync_new_block_boundary_commands(
                        &mut state,
                        &mut pending_routed,
                        &mut desired_execution_mode,
                        &mut execution_epoch,
                        &mut tainted_rebuild_parked,
                        &mut runtime_replay_active,
                        &mut replay_current_block_active,
                        &mut next_block_deadline,
                        &mut force_close,
                        &mut block_empty,
                        &mut l2_gas_consumed_block,
                        block_time,
                    )?,
                    NewBlockBoundarySyncOutcome::ContinueOuterLoop
                ) {
                    continue;
                }
                Self::normalize_routed_batch_for_execution_mode(&mut pending_routed, desired_execution_mode);
            }
            if let Some(block_n) = match &state {
                ExecutorThreadState::NewBlock(state_new_block) => Some(state_new_block.state_adaptor.block_n()),
                ExecutorThreadState::Executing(_) => None,
            } {
                if matches!(
                    self.wait_for_previous_block_close_or_command(
                        &mut state,
                        &mut pending_routed,
                        &mut desired_execution_mode,
                        &mut execution_epoch,
                        &mut tainted_rebuild_parked,
                        &mut runtime_replay_active,
                        &mut replay_current_block_active,
                        &mut next_block_deadline,
                        &mut force_close,
                        &mut block_empty,
                        &mut l2_gas_consumed_block,
                        block_time,
                        block_n,
                    )?,
                    NewBlockBoundarySyncOutcome::ContinueOuterLoop
                ) {
                    continue;
                }
            }
            let resolved_block_n_min_10_hash = if let Some(block_n) = match &state {
                ExecutorThreadState::NewBlock(state_new_block) => Some(state_new_block.state_adaptor.block_n()),
                ExecutorThreadState::Executing(_) => None,
            } {
                match self.wait_for_hash_of_block_min_10_or_command(
                    &mut state,
                    &mut pending_routed,
                    &mut desired_execution_mode,
                    &mut execution_epoch,
                    &mut tainted_rebuild_parked,
                    &mut runtime_replay_active,
                    &mut replay_current_block_active,
                    &mut next_block_deadline,
                    &mut force_close,
                    &mut block_empty,
                    &mut l2_gas_consumed_block,
                    block_time,
                    block_n,
                )? {
                    WaitForConfirmedHashOutcome::Hash(hash) => Some(hash),
                    WaitForConfirmedHashOutcome::ContinueOuterLoop => continue,
                }
            } else {
                None
            };

            let execution_state = match state {
                ExecutorThreadState::Executing(ref mut s) => s,
                ExecutorThreadState::NewBlock(state_new_block) => {
                    let execution_mode =
                        if pending_routed.is_empty() { desired_execution_mode } else { pending_routed.execution_mode };
                    replay_current_block_active = runtime_replay_active;
                    self.publish_replay_status(runtime_replay_active, execution_epoch);
                    self.publish_effective_execution_mode(execution_mode);
                    let create_state_start = Instant::now();
                    let execution_state = self
                        .create_execution_state(
                            state_new_block,
                            l2_gas_consumed_block,
                            execution_mode,
                            resolved_block_n_min_10_hash.expect("new-block hash wait result must be resolved"),
                        )
                        .context("Creating execution state")?;
                    l2_gas_consumed_block = 0;
                    tracing::debug!(
                        block_number = execution_state.exec_ctx.block_number,
                        create_execution_state_ms = create_state_start.elapsed().as_secs_f64() * 1000.0,
                        "executor_new_block_state_created"
                    );
                    tracing::debug!("Starting new block, block_n={}", execution_state.exec_ctx.block_number);
                    match self.send_forward_reply(
                        execution_epoch,
                        Some(execution_state.exec_ctx.block_number),
                        "StartNewBlock",
                        ExecutorMessage::StartNewBlock {
                            exec_ctx: execution_state.exec_ctx.clone(),
                            execution_mode: execution_state.execution_mode,
                            execution_epoch,
                        },
                    ) {
                        ForwardReplySendOutcome::Sent => {}
                        ForwardReplySendOutcome::DroppedStale => {
                            state = ExecutorThreadState::Executing(execution_state);
                            continue;
                        }
                        ForwardReplySendOutcome::ChannelClosed => break Ok(()),
                    }
                    state = ExecutorThreadState::Executing(execution_state);
                    let ExecutorThreadState::Executing(s) = &mut state else { unreachable!() };
                    s
                }
            };

            if execution_state.execution_mode == ExecutionMode::Mixed
                && execution_state.saw_blockifier_txs
                && self.replay_mode_enabled
                && self.replay_boundary_exists(execution_state.exec_ctx.block_number)
                && !pending_routed.rust_batch.is_empty()
            {
                let rescued_count = pending_routed.rust_batch.len();
                pending_routed.blockifier_batch.extend(mem::take(&mut pending_routed.rust_batch));
                tracing::debug!(
                    "executor_replay_blockifier_seen_rescuing_rust_batch block_number={} rescued_txs={} phase=before_blockifier",
                    execution_state.exec_ctx.block_number,
                    rescued_count
                );
            }

            // ── Phase A: Blockifier execution ────────────────────────────────────────
            // Apply replay boundary: cap how many blockifier txs we send to execute_txs.
            // Overflow stays in pending_routed.blockifier_batch for the next block.
            let blockifier_exec_count = if self.replay_mode_enabled {
                let block_n = execution_state.exec_ctx.block_number;
                if let Some(remaining) = self.replay_boundary_remaining_capacity(block_n) {
                    let count = pending_routed.blockifier_batch.len().min(remaining);
                    if count < pending_routed.blockifier_batch.len() {
                        tracing::debug!(
                            "replay_boundary_executor_slice block_number={} remaining_for_block={} moved_to_next_block={}",
                            block_n,
                            remaining,
                            pending_routed.blockifier_batch.len() - count
                        );
                    }
                    count
                } else {
                    pending_routed.blockifier_batch.len()
                }
            } else {
                pending_routed.blockifier_batch.len()
            };

            let (first_tx_hash, first_tx_type) = pending_routed
                .blockifier_batch
                .txs
                .first()
                .map(|tx| (format!("{:#x}", tx.tx_hash().to_felt()), tx_type_to_label(tx.tx_type())))
                .unwrap_or_else(|| ("none".to_string(), "none"));
            tracing::debug!(
                "executor_batch_execution_started block_number={} blockifier_txs={} rust_txs={} first_tx_hash={} first_tx_type={}",
                execution_state.exec_ctx.block_number,
                blockifier_exec_count,
                pending_routed.rust_batch.len(),
                first_tx_hash,
                first_tx_type
            );

            let mut combined_executed = BatchToExecute::default();
            let mut combined_results = Vec::new();
            let mut stats = ExecutionStats::default();
            let total_exec_start = Instant::now();

            // Phase A: run blockifier branch.
            let mut block_full = if blockifier_exec_count == 0 {
                // Skip Blockifier phase when the routed blockifier branch has nothing executable.
                false
            } else {
                let phase_start = Instant::now();
                let blockifier_results = execution_state
                    .executor
                    .execute_txs(&pending_routed.blockifier_batch.txs[..blockifier_exec_count], None);
                let phase_duration = phase_start.elapsed();

                let results_len = blockifier_results.len();
                let phase_block_full = results_len < blockifier_exec_count;
                let executed_b = pending_routed.blockifier_batch.remove_n_front(results_len);

                let avg_tx_time_ms =
                    if results_len > 0 { phase_duration.as_secs_f64() * 1000.0 / results_len as f64 } else { 0.0 };
                let mut replay_executed_hashes: Vec<Felt> = Vec::new();
                let mut blockifier_added = 0;
                for (btx, res) in executed_b.txs.iter().zip(blockifier_results.iter()) {
                    match res {
                        Ok((execution_info, _)) => {
                            let tx_execution_summary =
                                execution_info.summarize(execution_state.executor.block_context.versioned_constants());
                            let tx_builtin_counters = execution_info.summarize_builtins();
                            tracing::debug!(
                                tx_hash = format_args!("{:#x}", btx.tx_hash().to_felt()),
                                tx_summary = ?tx_execution_summary,
                                tx_builtin_counters = ?tx_builtin_counters,
                                tx_receipt_resources = ?execution_info.receipt.resources,
                                tx_receipt_gas = ?execution_info.receipt.gas,
                                "blockifier_exec_resources_after_tx"
                            );
                            tracing::trace!("Blockifier phase: executed tx {:#x}", btx.tx_hash().to_felt());
                            exec_metrics().record_tx_execution_time(
                                avg_tx_time_ms,
                                tx_type_to_label(btx.tx_type()),
                                context_label::PRODUCTION,
                            );
                            stats.n_added_to_block += 1;
                            blockifier_added += 1;
                            replay_executed_hashes.push(btx.tx_hash().to_felt());
                            stats.l2_gas_consumed += u128::from(execution_info.receipt.gas.l2_gas.0);
                            block_empty = false;
                            if execution_info.revert_error.is_some() {
                                stats.n_reverted += 1;
                            } else if let Some((class_hash, contract_class)) = btx.declared_contract_class() {
                                tracing::debug!("Declared class_hash={:#x}", class_hash.to_felt());
                                stats.declared_classes += 1;
                                execution_state.declared_classes.insert(class_hash, contract_class);
                            }
                        }
                        Err(err) => {
                            tracing::error!(
                                "Rejected transaction {:#x} for unexpected error: {err:#}",
                                btx.tx_hash().to_felt()
                            );
                            stats.n_rejected += 1;
                        }
                    }
                }
                stats.n_batches += 1;
                stats.n_executed += executed_b.len();
                stats.exec_duration += phase_duration;
                stats.n_added_by_blockifier += blockifier_added;
                stats.blockifier_exec_duration += phase_duration;

                self.record_replay_executed_hashes(execution_state.exec_ctx.block_number, &replay_executed_hashes);
                if results_len > 0 {
                    execution_state.saw_blockifier_txs = true;
                }

                combined_results.extend(blockifier_results);
                combined_executed.extend(executed_b);

                // T-033: phase metrics
                self.metrics
                    .executor_phase_duration_seconds
                    .record(phase_duration.as_secs_f64(), &[KeyValue::new("phase", "blockifier")]);
                self.metrics
                    .executor_phase_executed_txs_total
                    .add(results_len as u64, &[KeyValue::new("phase", "blockifier")]);
                if phase_block_full {
                    self.metrics.executor_block_full_total.add(1, &[KeyValue::new("phase", "blockifier")]);
                }

                tracing::debug!(
                    block_number = execution_state.exec_ctx.block_number,
                    txs_executed = results_len,
                    txs_added = blockifier_added,
                    txs_reverted = stats.n_reverted,
                    txs_rejected = stats.n_rejected,
                    duration_ms = phase_duration.as_secs_f64() * 1000.0,
                    avg_tx_ms = avg_tx_time_ms,
                    block_full = phase_block_full,
                    "executed_with_blockifier"
                );

                phase_block_full
            };

            // If blockifier phase filled the block, defer rust_batch to next block unchanged.
            // pending_routed retains: blockifier suffix (if any) + full rust_batch.
            if execution_state.execution_mode == ExecutionMode::Mixed
                && execution_state.saw_blockifier_txs
                && self.replay_mode_enabled
                && self.replay_boundary_exists(execution_state.exec_ctx.block_number)
                && !pending_routed.rust_batch.is_empty()
            {
                let rescued_count = pending_routed.rust_batch.len();
                pending_routed.blockifier_batch.extend(mem::take(&mut pending_routed.rust_batch));
                tracing::debug!(
                    "executor_replay_blockifier_seen_rescuing_rust_batch block_number={} rescued_txs={} phase=before_rust",
                    execution_state.exec_ctx.block_number,
                    rescued_count
                );
            }

            // ── Phase B: Rust execution (Mixed mode only) ────────────────────────────
            let execution_mode = execution_state.execution_mode;
            if !block_full {
                match execution_mode {
                    ExecutionMode::Mixed => {
                        if !pending_routed.rust_batch.is_empty() {
                            let phase_start = Instant::now();
                            let rust_output = mc_rust_exec::execute_txns(
                                &mut execution_state.executor,
                                &pending_routed.rust_batch.txs,
                                None,
                                &self.rust_exec_runtime_config,
                                &mut execution_state.rust_phase_state,
                            );
                            let phase_duration = phase_start.elapsed();

                            let results_len = rust_output.executed_count;
                            let rust_block_full = rust_output.block_full;
                            let executed_r = pending_routed.rust_batch.remove_n_front(results_len);

                            let avg_tx_time_ms = if results_len > 0 {
                                phase_duration.as_secs_f64() * 1000.0 / results_len as f64
                            } else {
                                0.0
                            };
                            let executed_r_len = executed_r.len();
                            let mut replay_executed_hashes: Vec<Felt> = Vec::new();
                            let mut rust_reverted: u64 = 0;
                            let mut rust_rejected: u64 = 0;
                            let mut rust_added = 0;
                            for (btx, res) in executed_r.txs.iter().zip(rust_output.results.iter()) {
                                match res {
                                    Ok((execution_info, _)) => {
                                        tracing::trace!("Rust phase: executed tx {:#x}", btx.tx_hash().to_felt());
                                        exec_metrics().record_tx_execution_time(
                                            avg_tx_time_ms,
                                            tx_type_to_label(btx.tx_type()),
                                            context_label::PRODUCTION,
                                        );
                                        stats.n_added_to_block += 1;
                                        rust_added += 1;
                                        replay_executed_hashes.push(btx.tx_hash().to_felt());
                                        stats.l2_gas_consumed += u128::from(execution_info.receipt.gas.l2_gas.0);
                                        block_empty = false;
                                        if execution_info.revert_error.is_some() {
                                            stats.n_reverted += 1;
                                            rust_reverted += 1;
                                        } else if let Some((class_hash, contract_class)) = btx.declared_contract_class()
                                        {
                                            stats.declared_classes += 1;
                                            execution_state.declared_classes.insert(class_hash, contract_class);
                                        }
                                    }
                                    Err(err) => {
                                        tracing::error!("Rejected rust tx {:#x}: {err:#}", btx.tx_hash().to_felt());
                                        stats.n_rejected += 1;
                                        rust_rejected += 1;
                                    }
                                }
                            }
                            stats.n_executed += executed_r_len;
                            stats.exec_duration += phase_duration;
                            stats.n_added_by_rust_exec += rust_added;
                            stats.rust_exec_duration += phase_duration;

                            self.record_replay_executed_hashes(
                                execution_state.exec_ctx.block_number,
                                &replay_executed_hashes,
                            );

                            combined_results.extend(rust_output.results);
                            combined_executed.extend(executed_r);

                            // T-033: rust phase metrics
                            self.metrics
                                .executor_phase_duration_seconds
                                .record(phase_duration.as_secs_f64(), &[KeyValue::new("phase", "rust")]);
                            self.metrics
                                .executor_phase_executed_txs_total
                                .add(results_len as u64, &[KeyValue::new("phase", "rust")]);
                            tracing::debug!(
                                target: "RUST_EXEC",
                                block_number = execution_state.exec_ctx.block_number,
                                txs_executed = results_len,
                                txs_added = rust_added,
                                txs_reverted = rust_reverted,
                                txs_rejected = rust_rejected,
                                duration_ms = phase_duration.as_secs_f64() * 1000.0,
                                avg_tx_ms = avg_tx_time_ms,
                                block_full = rust_block_full,
                                mode = ?execution_mode,
                                "executed_with_rust_exec"
                            );

                            if matches!(rust_output.deferred_reason, Some(RustDeferredReason::UnsupportedOrFailed)) {
                                let rescued_count = pending_routed.rust_batch.len();
                                pending_routed.blockifier_batch.extend(mem::take(&mut pending_routed.rust_batch));
                                desired_execution_mode = ExecutionMode::BlockifierOnly;
                                self.publish_effective_execution_mode(desired_execution_mode);
                                pending_routed.execution_mode = execution_state.execution_mode;
                                tracing::error!(
                                    block_number = execution_state.exec_ctx.block_number,
                                    rescued_count,
                                    "executor_rust_failure_rescued_remaining_txs_to_blockifier"
                                );
                                self.metrics
                                    .executor_rust_capacity_reject_total
                                    .add(1, &[KeyValue::new("reason", "unsupported_or_failed")]);
                            } else if rust_block_full {
                                block_full = true;
                                // pending_routed.rust_batch retains deferred suffix
                                self.metrics.executor_block_full_total.add(1, &[KeyValue::new("phase", "rust")]);
                                let reason = match rust_output.deferred_reason {
                                    Some(RustDeferredReason::Capacity) => "limit_exceeded",
                                    Some(RustDeferredReason::ResourceError) => "resource_error",
                                    Some(RustDeferredReason::UnsupportedOrFailed) => "unsupported_or_failed",
                                    None => "limit_exceeded",
                                };
                                self.metrics
                                    .executor_rust_capacity_reject_total
                                    .add(1, &[KeyValue::new("reason", reason)]);
                            }
                        }
                    }
                    ExecutionMode::BlockifierOnly => {
                        // C-022: Rescue deferred rust txs on mode transition.
                        // When mode switches to BlockifierOnly (e.g., after comparator stop),
                        // deferred rust txs must be moved to blockifier_batch so they get
                        // replayed via blockifier. Clearing them would silently drop txs
                        // that already left the mempool.
                        if !pending_routed.rust_batch.is_empty() {
                            let rescued_count = pending_routed.rust_batch.len();
                            pending_routed.blockifier_batch.extend(mem::take(&mut pending_routed.rust_batch));
                            tracing::debug!(
                                rescued_count,
                                "executor_rescued_deferred_rust_txs_to_blockifier_on_mode_transition"
                            );
                        }
                    }
                }
            }

            let exec_duration = total_exec_start.elapsed();
            l2_gas_consumed_block += stats.l2_gas_consumed;

            tracing::debug!("Finished batch execution.");
            tracing::debug!("Stats: {:?}", stats);
            tracing::debug!(
                "Weights: {:?}",
                execution_state.executor.bouncer.lock().expect("Bouncer lock poisoned").get_bouncer_weights()
            );
            tracing::debug!("Block now full: {:?}", block_full);
            if let Some(block_state) = execution_state.executor.block_state.as_mut() {
                block_state.state.evict_read_cache_if_needed();
            }

            execution_state.executed_in_block.extend(combined_executed.clone());

            let processed_original_hashes = pending_routed.take_processed_original_hashes(&combined_executed.txs);
            let mut successful_hashes: std::collections::BTreeMap<Felt, usize> = combined_executed
                .txs
                .iter()
                .zip(&combined_results)
                .filter_map(|(tx, result)| result.is_ok().then_some(tx.tx_hash().to_felt()))
                .fold(std::collections::BTreeMap::new(), |mut counts, hash| {
                    *counts.entry(hash).or_default() += 1usize;
                    counts
                });
            let original_tx_hashes = processed_original_hashes
                .into_iter()
                .filter(|hash| {
                    successful_hashes.get_mut(hash).is_some_and(|count| {
                        let include = *count > 0;
                        *count = count.saturating_sub(1);
                        include
                    })
                })
                .collect();

            // One combined BatchExecuted per routed payload (design doc §"one combined BatchExecuted").
            let exec_result = BatchExecutionResult {
                executed_txs: combined_executed,
                original_tx_hashes,
                blockifier_results: combined_results,
                stats,
                execution_mode: execution_state.execution_mode,
                execution_epoch,
                emitted_at: StdInstant::now(),
            };
            tracing::debug!(
                "executor_batch_execution_finished block_number={} txs_executed={} txs_added_to_block={} txs_reverted={} txs_rejected={} batch_exec_duration_ms={} block_full={} first_tx_hash={} first_tx_type={}",
                execution_state.exec_ctx.block_number,
                exec_result.stats.n_executed,
                exec_result.stats.n_added_to_block,
                exec_result.stats.n_reverted,
                exec_result.stats.n_rejected,
                exec_duration.as_secs_f64() * 1000.0,
                block_full,
                first_tx_hash,
                first_tx_type
            );
            if exec_result.stats.n_executed > 0 {
                match self.send_forward_reply(
                    execution_epoch,
                    Some(execution_state.exec_ctx.block_number),
                    "BatchExecuted",
                    ExecutorMessage::BatchExecuted(exec_result),
                ) {
                    ForwardReplySendOutcome::Sent => {}
                    ForwardReplySendOutcome::DroppedStale => continue,
                    ForwardReplySendOutcome::ChannelClosed => break Ok(()),
                }
            }

            // ── Close condition ──────────────────────────────────────────────────────
            let block_n = execution_state.exec_ctx.block_number;
            let now = Instant::now();
            let block_time_deadline_reached = now >= next_block_deadline;
            let replay_boundary_exists = self.replay_mode_enabled && self.replay_boundary_exists(block_n);
            let replay_boundary_met =
                if replay_boundary_exists { self.replay_boundary_is_met(block_n).unwrap_or(false) } else { false };

            let should_close = if replay_boundary_exists {
                force_close || replay_boundary_met
            } else {
                force_close || block_full || block_time_deadline_reached
            };

            if should_close {
                // T-033: emit deferred_txs metrics for any pending suffixes being carried over.
                let deferred_reason = if force_close {
                    "force_close"
                } else if block_full {
                    "block_full"
                } else {
                    "deadline_close"
                };
                let deferred_blockifier = pending_routed.blockifier_batch.len() as u64;
                let deferred_rust = pending_routed.rust_batch.len() as u64;
                if deferred_blockifier > 0 {
                    self.metrics.executor_phase_deferred_txs_total.add(
                        deferred_blockifier,
                        &[KeyValue::new("phase", "blockifier"), KeyValue::new("reason", deferred_reason)],
                    );
                }
                if deferred_rust > 0 {
                    self.metrics.executor_phase_deferred_txs_total.add(
                        deferred_rust,
                        &[KeyValue::new("phase", "rust"), KeyValue::new("reason", deferred_reason)],
                    );
                }

                tracing::debug!(
                    "Ending block block_n={} (force_close={force_close}, block_full={block_full}, block_time_deadline_reached={block_time_deadline_reached}, replay_boundary_exists={replay_boundary_exists}, replay_boundary_met={replay_boundary_met})",
                    block_n,
                );
                let finalize_start = Instant::now();
                let mut block_exec_summary = execution_state.executor.finalize()?;
                block_exec_summary.bouncer_weights =
                    execution_state.rust_phase_state.effective_bouncer_weights(block_exec_summary.bouncer_weights);
                let finalize_secs = finalize_start.elapsed().as_secs_f64();
                self.metrics.executor_finalize_duration.record(finalize_secs, &[]);
                self.metrics.executor_finalize_last.record(finalize_secs, &[]);
                tracing::debug!(
                    block_number = block_n,
                    finalize_ms = finalize_secs * 1000.0,
                    "executor_finalize_complete"
                );

                match self.send_forward_reply(
                    execution_epoch,
                    Some(block_n),
                    "EndBlock",
                    ExecutorMessage::EndBlock {
                        block_exec_summary: Box::new(block_exec_summary),
                        block_number: block_n,
                        execution_epoch,
                    },
                ) {
                    ForwardReplySendOutcome::Sent => {}
                    ForwardReplySendOutcome::DroppedStale => continue,
                    ForwardReplySendOutcome::ChannelClosed => break Ok(()),
                }
                tracing::debug!(
                    block_number = block_n,
                    force_close = force_close,
                    block_full = block_full,
                    block_time_deadline_reached = block_time_deadline_reached,
                    replay_boundary_exists = replay_boundary_exists,
                    replay_boundary_met = replay_boundary_met,
                    "executor_end_block_sent"
                );
                next_block_deadline = Instant::now() + block_time;
                let end_block_start = Instant::now();
                state = self.end_block(execution_state).context("Ending block")?;
                Self::rebind_routed_batch_to_block(&mut pending_routed, Self::current_executor_block_n(&state));
                self.publish_effective_execution_mode(desired_execution_mode);
                if replay_current_block_active {
                    replay_current_block_active = false;
                    if pending_routed.is_empty() {
                        runtime_replay_active = false;
                    }
                    self.publish_replay_status(runtime_replay_active, execution_epoch);
                }
                tracing::debug!(
                    end_block_ms = end_block_start.elapsed().as_secs_f64() * 1000.0,
                    "executor_end_block_state_transition_complete"
                );
                block_empty = true;
                force_close = false;
                // pending_routed naturally carries deferred suffixes to the next block.
            }
        }
    }

    fn finish_shutdown(&mut self, state: ExecutorThreadState, execution_epoch: u64) {
        match state {
            ExecutorThreadState::Executing(mut execution_state) => {
                tracing::debug!(
                    "Shutting down executor, closing block block_n={}",
                    execution_state.exec_ctx.block_number
                );
                let finalize_start = Instant::now();
                match execution_state.executor.finalize() {
                    Ok(block_exec_summary) => {
                        let finalize_secs = finalize_start.elapsed().as_secs_f64();
                        self.metrics.executor_finalize_duration.record(finalize_secs, &[]);
                        self.metrics.executor_finalize_last.record(finalize_secs, &[]);
                        if self
                            .replies_sender
                            .blocking_send(ExecutorMessage::EndFinalBlock {
                                block_exec_summary: Some(Box::new(block_exec_summary)),
                                block_number: Some(execution_state.exec_ctx.block_number),
                                execution_epoch,
                            })
                            .is_err()
                        {
                            tracing::warn!(
                                "Could not send EndFinalBlock during shutdown, block will remain preconfirmed"
                            );
                        }
                    }
                    Err(e) => {
                        if self
                            .replies_sender
                            .blocking_send(ExecutorMessage::EndFinalBlock {
                                block_exec_summary: None,
                                block_number: Some(execution_state.exec_ctx.block_number),
                                execution_epoch,
                            })
                            .is_err()
                        {
                            tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
                        }
                        tracing::warn!(
                            "Failed to finalize block during shutdown: {:?}. Block will remain preconfirmed",
                            e
                        );
                    }
                }
            }
            ExecutorThreadState::NewBlock(_) => {
                tracing::debug!("Shutting down executor, no block to close");
                if self
                    .replies_sender
                    .blocking_send(ExecutorMessage::EndFinalBlock {
                        block_exec_summary: None,
                        block_number: None,
                        execution_epoch,
                    })
                    .is_err()
                {
                    tracing::warn!("Could not send EndFinalBlock(None) during shutdown");
                }
            }
        }
    }
}
