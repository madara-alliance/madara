use super::*;

impl BlockProductionTask {
    fn sorted_felt_hex_values(values: &HashSet<Felt>) -> Vec<String> {
        let mut values = values.iter().map(|value| format!("{:#x}", value)).collect::<Vec<_>>();
        values.sort();
        values
    }

    fn log_rust_exec_startup_config(&self) {
        let executor_addresses = Self::sorted_felt_hex_values(&self.routing_cfg.executor_addresses);
        let supported_selectors = Self::sorted_felt_hex_values(&self.routing_cfg.supported_selectors);
        let supported_class_hashes = Self::sorted_felt_hex_values(&self.routing_cfg.supported_class_hashes);
        let ignored_storage_addresses = Self::comparator_ignored_storage_addresses(
            self.backend.chain_config(),
            self.routing_cfg.runtime_options.ignore_fee_token_mismatch,
        );
        let mut ignored_storage_addresses =
            ignored_storage_addresses.iter().map(|address| format!("{:#x}", address)).collect::<Vec<_>>();
        ignored_storage_addresses.sort();

        let (status, summary) = match self.fallback.startup_mode {
            StartupExecutionMode::Mixed => ("✅ enabled", "selected transactions will use rust exec"),
            StartupExecutionMode::BlockifierOnly => ("⏸️ disabled", "all transactions will route via Cairo"),
        };
        tracing::info!(target: "RUST_EXEC", "{} - {}", status, summary);

        tracing::info!(
            target: "RUST_EXEC",
            "startup_config\n  startup_mode={:?}\n  effective_mode={:?}\n  startup_recovery_active={}\n  comparator_enabled={}\n  replay_mode_enabled={}\n  rust_batch_size={}\n  blockifier_batch_size={}\n  executor_addresses_count={}\n  executor_addresses={:?}\n  supported_selectors_source=supported_contracts.json\n  supported_selectors_count={}\n  supported_selectors={:?}\n  supported_class_hashes_source=supported_contracts.json\n  supported_class_hashes_count={}\n  supported_class_hashes={:?}\n  ignored_storage_addresses={:?}\n  no_charge_fee={}\n  conversion_log={}\n  execution_log={}\n  execution_log_inner={}\n  tx_diff_log={}\n  debug_block={:?}\n  inner_timing_log={}\n  ctx_cache={}\n  pedersen_cache={}\n  precomputed_sn_keccak={}\n  hash_agg_logs={}\n  storage_agg_logs={}\n  ignore_fee_mismatch={}\n  ignore_fee_token_mismatch={}\n  ignored_storage_mismatch_canonical_source={:?}\n  settle_trade_v3_positions={:?}",
            self.fallback.startup_mode,
            self.fallback.mode,
            self.fallback.startup_recovery_active,
            self.fallback.comparator_enabled,
            self.replay_mode_enabled,
            self.routing_cfg.rust_batch_size,
            self.routing_cfg.blockifier_batch_size,
            executor_addresses.len(),
            executor_addresses,
            supported_selectors.len(),
            supported_selectors,
            supported_class_hashes.len(),
            supported_class_hashes,
            ignored_storage_addresses,
            self.no_charge_fee,
            self.routing_cfg.runtime_options.conversion_log,
            self.routing_cfg.runtime_options.execution_log,
            self.routing_cfg.runtime_options.execution_log_inner,
            self.routing_cfg.runtime_options.tx_diff_log,
            self.routing_cfg.runtime_options.debug_block,
            self.routing_cfg.runtime_options.inner_timing_log,
            self.routing_cfg.runtime_options.ctx_cache,
            self.routing_cfg.runtime_options.pedersen_cache,
            self.routing_cfg.runtime_options.precomputed_sn_keccak,
            self.routing_cfg.runtime_options.hash_agg_logs,
            self.routing_cfg.runtime_options.storage_agg_logs,
            self.routing_cfg.runtime_options.ignore_fee_mismatch,
            self.routing_cfg.runtime_options.ignore_fee_token_mismatch,
            self.routing_cfg.runtime_options.ignored_storage_mismatch_canonical_source,
            self.routing_cfg.runtime_options.settle_trade_v3_positions,
        );
    }

    pub(super) async fn maybe_process_ready_priority_work(
        &mut self,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<bool> {
        if self.canonicalization_task.as_ref().is_some_and(|task| task.is_finished()) {
            let canonicalization_res = {
                let task = self.canonicalization_task.as_mut().expect("checked in guard");
                task.await
            };
            self.canonicalization_task = None;
            let result = canonicalization_res.context("Canonicalization task join failed")??;
            self.handle_canonicalization_result(result, close_queue)
                .await
                .context("Processing canonicalization completion")?;
            self.maybe_start_canonicalization_task();
            return Ok(true);
        }

        if self.pending_stop_fallback_handoff.as_ref().is_some_and(|pending| pending.carry.carry_task.is_finished()) {
            let carry_res = {
                let pending = self.pending_stop_fallback_handoff.as_mut().expect("checked in guard");
                (&mut pending.carry.carry_task).await
            };
            let carry_res = carry_res.context("Strict stop fallback carry task join failed")??;
            self.finish_pending_stop_fallback_handoff(close_queue, carry_res)
                .await
                .context("Completing strict stop fallback handoff")?;
            return Ok(true);
        }

        if self.tainted_rebuild_task.as_ref().is_some_and(|task| task.is_finished()) {
            let tainted_rebuild_res = {
                let task = self.tainted_rebuild_task.as_mut().expect("checked in guard");
                task.await
            };
            self.tainted_rebuild_task = None;
            let result = tainted_rebuild_res.context("Tainted rebuild task join failed")??;
            self.tainted_rebuild_session = result.live_session_after_step;
            self.publish_tainted_rebuild_gate();
            let TaintedRebuildClosePayload {
                state,
                canonical_bouncer_weights,
                state_diff,
                canonical_executed_rows,
                canonical_header,
            } = result.close_payload;
            self.enqueue_canonical_close_payload(
                close_queue,
                state,
                canonical_bouncer_weights,
                state_diff,
                canonical_executed_rows,
                canonical_header,
            )
            .context("Enqueueing tainted rebuild close payload")?;
            return Ok(true);
        }

        if !self.pending_completions.is_empty() {
            let completion = {
                let (_block_n, rx) = self.pending_completions.front_mut().expect("checked non-empty");
                match rx.try_recv() {
                    Ok(completion) => Some(Ok(completion)),
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => None,
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        Some(Err(anyhow::anyhow!("Close queue worker dropped completion channel")))
                    }
                }
            };
            if let Some(completion) = completion {
                let (expected_block_n, _rx) = self.pending_completions.pop_front().expect("pending completion exists");
                let completion = completion??;
                self.handle_close_completion(close_queue, expected_block_n, completion)?;
                return Ok(true);
            }
        }

        Ok(false)
    }

    pub(super) async fn process_reply(
        &mut self,
        reply: ExecutorMessage,
        close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        match reply {
            ExecutorMessage::StartNewBlock { exec_ctx, execution_mode, execution_epoch } => {
                if execution_epoch != self.execution_epoch {
                    tracing::info!(
                        block_number = exec_ctx.block_number,
                        message_epoch = execution_epoch,
                        current_epoch = self.execution_epoch,
                        "dropping_stale_start_new_block_after_fallback"
                    );
                    return Ok(());
                }
                tracing::debug!("Received ExecutorMessage::StartNewBlock block_n={}", exec_ctx.block_number);
                let current_state = self.current_state.take().context("No current state")?;
                let TaskState::NotExecuting { latest_block_n } = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be NotExecuting")
                };

                let expected_block_n = self.next_internal_preconfirmed_block_n_from_backend()?;
                if exec_ctx.block_number < expected_block_n {
                    tracing::info!(
                        block_number = exec_ctx.block_number,
                        expected_block_n,
                        latest_block_n = ?latest_block_n,
                        "dropping_redundant_start_new_block_after_head_advance"
                    );
                    self.current_state = Some(TaskState::NotExecuting { latest_block_n });
                    return Ok(());
                }

                if expected_block_n != exec_ctx.block_number {
                    anyhow::bail!(
                        "Received new block_n={} from executor, expected block_n={} from backend head (task latest_block_n={latest_block_n:?})",
                        exec_ctx.block_number,
                        expected_block_n
                    )
                }

                let backend = self.backend.clone();
                global_spawn_rayon_task(move || {
                    backend.write_access().new_preconfirmed(PreconfirmedBlock::new(exec_ctx.into_header()))
                })
                .await?;

                self.current_state = Some(TaskState::Executing(CurrentBlockState::with_execution_mode(
                    self.backend.clone(),
                    expected_block_n,
                    execution_mode,
                )));
                self.record_block_stage_metrics();
            }
            ExecutorMessage::BatchExecuted(batch_execution_result) => {
                if batch_execution_result.execution_epoch != self.execution_epoch {
                    tracing::info!(
                        message_epoch = batch_execution_result.execution_epoch,
                        current_epoch = self.execution_epoch,
                        execution_mode = ?batch_execution_result.execution_mode,
                        txs_executed_in_batch = batch_execution_result.stats.n_executed,
                        "dropping_stale_batch_executed_after_fallback"
                    );
                    return Ok(());
                }

                let current_state = self.current_state.as_mut().context("No current state")?;
                let TaskState::Executing(state) = current_state else {
                    anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
                };
                let batch_exec_secs = batch_execution_result.stats.exec_duration.as_secs_f64();
                let batch_exec_ms = batch_exec_secs * 1000.0;
                let executor_to_main_delivery_secs = batch_execution_result.emitted_at.elapsed().as_secs_f64();
                let executor_to_main_delivery_ms = executor_to_main_delivery_secs * 1000.0;
                self.metrics.executor_batch_execution_duration.record(batch_exec_secs, &[]);
                self.metrics.executor_batch_execution_last.record(batch_exec_secs, &[]);
                self.metrics.executor_to_main_delivery_duration.record(executor_to_main_delivery_secs, &[]);
                self.metrics.executor_to_main_delivery_last.record(executor_to_main_delivery_secs, &[]);
                tracing::debug!(
                    "received_executor_batch_executed block_number={} txs_executed_in_batch={} txs_added_to_block={} txs_reverted={} txs_rejected={} batch_exec_duration_ms={} executor_to_main_delivery_ms={} close_queue_depth={} close_queue_in_flight={} pending_close_completions={}",
                    state.block_number,
                    batch_execution_result.stats.n_executed,
                    batch_execution_result.stats.n_added_to_block,
                    batch_execution_result.stats.n_reverted,
                    batch_execution_result.stats.n_rejected,
                    batch_exec_ms,
                    executor_to_main_delivery_ms,
                    close_queue.current_depth(),
                    close_queue.current_in_flight(),
                    self.pending_completions.len()
                );

                self.metrics.record_execution_stats(&batch_execution_result.stats);
                state.accumulated_stats = state.accumulated_stats.clone() + batch_execution_result.stats.clone();
                state.append_batch(batch_execution_result).await?;
                self.send_state_notification(BlockProductionStateNotification::BatchExecuted);
            }
            ExecutorMessage::EndBlock { block_exec_summary, block_number, execution_epoch } => {
                if execution_epoch != self.execution_epoch {
                    tracing::info!(
                        block_number,
                        message_epoch = execution_epoch,
                        current_epoch = self.execution_epoch,
                        "dropping_stale_end_block_after_fallback"
                    );
                    return Ok(());
                }

                match self.current_state.as_ref() {
                    Some(TaskState::Executing(state)) => {
                        if block_number < state.block_number {
                            tracing::info!(
                                block_number,
                                current_block_n = state.block_number,
                                "dropping_redundant_end_block_behind_current_execution"
                            );
                            return Ok(());
                        }
                        if block_number > state.block_number {
                            anyhow::bail!(
                                "Received end block_n={} from executor, but current executing block_n={} is older",
                                block_number,
                                state.block_number
                            );
                        }
                    }
                    Some(TaskState::NotExecuting { .. }) => {
                        let expected_block_n = self.next_internal_preconfirmed_block_n_from_backend()?;
                        if block_number < expected_block_n {
                            tracing::info!(
                                block_number,
                                expected_block_n,
                                "dropping_redundant_end_block_after_head_advance"
                            );
                            return Ok(());
                        }
                    }
                    None => anyhow::bail!("No current state"),
                }

                tracing::info!("received_executor_end_block");
                self.close_block(block_exec_summary, close_queue).await?;
            }
            ExecutorMessage::EndFinalBlock { block_exec_summary, block_number, execution_epoch } => {
                if execution_epoch != self.execution_epoch {
                    tracing::info!(
                        block_number = ?block_number,
                        message_epoch = execution_epoch,
                        current_epoch = self.execution_epoch,
                        "dropping_stale_end_final_block_after_fallback"
                    );
                    return Ok(());
                }

                if let Some(block_number) = block_number {
                    match self.current_state.as_ref() {
                        Some(TaskState::Executing(state)) => {
                            if block_number < state.block_number {
                                tracing::info!(
                                    block_number,
                                    current_block_n = state.block_number,
                                    "dropping_redundant_end_final_block_behind_current_execution"
                                );
                                return Ok(());
                            }
                            if block_number > state.block_number {
                                anyhow::bail!(
                                    "Received final end block_n={} from executor, but current executing block_n={} is older",
                                    block_number,
                                    state.block_number
                                );
                            }
                        }
                        Some(TaskState::NotExecuting { .. }) => {
                            let expected_block_n = self.next_internal_preconfirmed_block_n_from_backend()?;
                            if block_number < expected_block_n {
                                tracing::info!(
                                    block_number,
                                    expected_block_n,
                                    "dropping_redundant_end_final_block_after_head_advance"
                                );
                                return Ok(());
                            }
                        }
                        None => anyhow::bail!("No current state"),
                    }
                }

                tracing::debug!("Received ExecutorMessage::EndFinalBlock (shutdown)");
                match block_exec_summary {
                    Some(summary) => {
                        self.close_block(summary, close_queue).await?;
                    }
                    None => {
                        tracing::debug!("EndFinalBlock(None) received - executor completed without block");
                    }
                }
            }
        }

        Ok(())
    }

    pub(super) fn executor_replay_status(&self) -> crate::fallback::types::RuntimeReplayStatus {
        *self.executor_replay_status_rx.borrow()
    }

    pub(super) fn control_plane_replay_status(&self) -> crate::fallback::types::RuntimeReplayStatus {
        if self.tainted_rebuild_active() {
            return crate::fallback::types::RuntimeReplayStatus::in_progress_for_epoch(self.execution_epoch);
        }
        let executor_status = self.executor_replay_status();
        if executor_status.execution_epoch < self.execution_epoch {
            crate::fallback::types::RuntimeReplayStatus::in_progress_for_epoch(self.execution_epoch)
        } else {
            executor_status
        }
    }

    pub(super) fn executionbox_status_snapshot(&self) -> crate::fallback::types::ExecutionboxStatus {
        self.fallback.status(self.control_plane_replay_status())
    }

    pub(super) fn executionbox_enable_from_control_plane(
        &mut self,
    ) -> Result<crate::fallback::manager::EnableOutcome, crate::fallback::manager::EnableError> {
        use crate::fallback::manager::EnableError;

        if self.fallback.startup_recovery_active {
            return Err(EnableError::ReplayInProgress);
        }

        if !self.control_plane_replay_status().replay_backlog_empty {
            return Err(EnableError::ReplayInProgress);
        }

        self.fallback.executionbox_enable()
    }

    #[tracing::instrument(skip(self, ctx), fields(module = "BlockProductionTask"))]
    pub async fn run(mut self, ctx: ServiceContext) -> Result<(), anyhow::Error> {
        self.setup_initial_state().await?;
        self.metrics.close_queue_depth.record(0, &[]);
        self.metrics.close_queue_in_flight.record(0, &[]);
        self.record_block_stage_metrics();
        self.log_rust_exec_startup_config();

        let mut executor = crate::executor::start_executor_thread(
            Arc::clone(&self.backend),
            self.executor_commands_recv.take().context("Task already started")?,
            self.metrics.clone(),
            self.replay_mode_enabled,
            self.executor_replay_status_tx.clone(),
            self.execution_mode_tx.clone(),
            self.execution_mode_rx.clone(),
            self.execution_epoch_rx.clone(),
            crate::util::build_rust_exec_runtime_config(&self.routing_cfg, self.no_charge_fee),
        )
        .context("Starting executor thread")?;

        if let Some(session) = self.tainted_rebuild_session.as_ref() {
            self.handle
                .resync_to_backend_head(Some(session.next_block_n))
                .context("Queueing executor resync for startup tainted rebuild replay")?;
        }

        let mut fallback_commands_recv =
            self.fallback_commands_recv.take().context("Fallback commands channel already taken")?;

        let close_queue_capacity = self.close_queue_capacity();
        let (close_queue_handle, finalizer_task_handle) =
            FinalizerHandle::spawn(close_queue_capacity, self.metrics.clone(), Self::execute_close_payload);
        tracing::info!(
            "initialized_finalizer_runtime mode=serial queue_capacity={} configured_capacity={}",
            close_queue_capacity,
            close_queue_handle.configured_capacity()
        );

        let batch_sender = executor.send_batch.take().context("Channel sender already taken")?;
        let bypass_tx_input = self.bypass_tx_input.take().context("Bypass tx channel already taken")?;
        let mut batcher_task = AbortOnDrop::spawn(
            Batcher::new(
                self.backend.clone(),
                self.mempool.clone(),
                self.l1_client.clone(),
                ctx,
                batch_sender,
                bypass_tx_input,
                self.mempool_intake_rx.clone(),
                self.handle.mempool_intake_tx(),
                self.tainted_rebuild_active_rx.clone(),
                self.execution_mode_rx.clone(),
                self.execution_epoch_rx.clone(),
                self.routing_cfg.clone(),
                self.metrics.clone(),
                self.replay_mode_enabled,
            )
            .run(),
        );

        let mut batcher_completed = false;
        let mut end_final_block_received = false;
        let mut executor_stopped = false;
        let mut batcher_error: Option<anyhow::Error> = None;

        let loop_result: anyhow::Result<()> = loop {
            if self
                .maybe_process_ready_priority_work(&close_queue_handle)
                .await
                .context("Processing ready high-priority block production work")?
            {
                continue;
            }

            self.maybe_start_tainted_rebuild_task().context("Starting tainted rebuild step")?;

            tokio::select! {
                res = &mut batcher_task, if !batcher_completed => {
                    batcher_completed = true;
                    match res {
                        Ok(()) => tracing::debug!("Batcher task completed normally"),
                        Err(e) => {
                            let error = e.context("In batcher task");
                            tracing::warn!("Batcher task errored: {error:?}");
                            batcher_error = Some(error);
                            if self.backend.has_preconfirmed_block() {
                                tracing::warn!("Batcher errored with preconfirmed block, attempting graceful shutdown");
                            }
                        }
                    }
                }

                Some(cmd) = fallback_commands_recv.recv() => {
                    use crate::fallback::manager::{EnableError, EnableOutcome};
                    use crate::fallback::types::ExecutionMode;
                    use opentelemetry::KeyValue;

                    match cmd {
                        crate::executor::FallbackCommand::Enable(resp) => {
                            let result = self.executionbox_enable_from_control_plane();
                            self.metrics.executionbox_manual_enable_total.add(1, &[]);
                            let result_label = match &result {
                                Ok(EnableOutcome::EnabledNow) => "enabled_now",
                                Ok(EnableOutcome::AlreadyMixed) => "already_mixed",
                                Err(EnableError::ReplayInProgress) => "replay_in_progress",
                            };
                            self.metrics.executionbox_enable_result_total.add(
                                1,
                                &[KeyValue::new("result", result_label)],
                            );
                            let mode_val: u64 = if self.fallback.mode == ExecutionMode::Mixed { 1 } else { 0 };
                            self.metrics.execution_mode_current.record(mode_val, &[]);
                            if let Err(err) = self.handle.set_desired_execution_mode(self.fallback.mode) {
                                tracing::warn!("Failed to propagate desired execution mode to executor: {err:#}");
                            }
                            tracing::info!("executionbox_enable result={result_label}");
                            let _ = resp.send(result);
                        }
                        crate::executor::FallbackCommand::Disable(resp) => {
                            self.fallback.executionbox_disable();
                            self.metrics.executionbox_manual_disable_total.add(1, &[]);
                            self.metrics.execution_mode_current.record(0, &[]);
                            if let Err(err) = self.handle.set_desired_execution_mode(self.fallback.mode) {
                                tracing::warn!("Failed to propagate desired execution mode to executor: {err:#}");
                            }
                            tracing::info!("executionbox_disable applied mode={:?}", self.fallback.mode);
                            let _ = resp.send(());
                        }
                        crate::executor::FallbackCommand::Status(resp) => {
                            let _ = resp.send(self.executionbox_status_snapshot());
                        }
                    }
                }

                Some(reply) = executor.replies.recv() => {
                    let is_end_final_block = matches!(reply, ExecutorMessage::EndFinalBlock { .. });
                    if let Err(e) = self.process_reply(reply, &close_queue_handle)
                        .await
                        .context("Processing reply from executor thread")
                    {
                        break Err(e);
                    }
                    if is_end_final_block {
                        end_final_block_received = true;
                        tracing::debug!("EndFinalBlock processed, executor completed");
                    }
                }

                canonicalization_res = async {
                    let task = self.canonicalization_task.as_mut().expect("checked in guard");
                    task.await
                }, if self.canonicalization_task.is_some() => {
                    self.canonicalization_task = None;
                    let result = canonicalization_res.context("Canonicalization task join failed")??;
                    self.handle_canonicalization_result(result, &close_queue_handle)
                        .await
                        .context("Processing canonicalization completion")?;
                    self.maybe_start_canonicalization_task();
                }

                carry_res = async {
                    let pending = self.pending_stop_fallback_handoff.as_mut().expect("checked in guard");
                    (&mut pending.carry.carry_task).await
                }, if self.pending_stop_fallback_handoff.is_some() => {
                    let carry_res = carry_res.context("Strict stop fallback carry task join failed")??;
                    self.finish_pending_stop_fallback_handoff(&close_queue_handle, carry_res)
                        .await
                        .context("Completing strict stop fallback handoff")?;
                }

                tainted_rebuild_res = async {
                    let task = self.tainted_rebuild_task.as_mut().expect("checked in guard");
                    task.await
                }, if self.tainted_rebuild_task.is_some() => {
                    self.tainted_rebuild_task = None;
                    let result = tainted_rebuild_res.context("Tainted rebuild task join failed")??;
                    self.tainted_rebuild_session = result.live_session_after_step;
                    self.publish_tainted_rebuild_gate();
                    let TaintedRebuildClosePayload {
                        state,
                        canonical_bouncer_weights,
                        state_diff,
                        canonical_executed_rows,
                        canonical_header,
                    } = result.close_payload;
                    self.enqueue_canonical_close_payload(
                        &close_queue_handle,
                        state,
                        canonical_bouncer_weights,
                        state_diff,
                        canonical_executed_rows,
                        canonical_header,
                    )
                    .context("Enqueueing tainted rebuild close payload")?;
                }

                completion_res = async {
                    let (_block_n, rx) = self.pending_completions.front_mut().expect("checked non-empty");
                    rx.await
                }, if !self.pending_completions.is_empty() => {
                    let (expected_block_n, _rx) = self.pending_completions.pop_front().expect("pending completion exists");
                    let completion = completion_res
                        .context("Close queue worker dropped completion channel")??;
                    self.handle_close_completion(&close_queue_handle, expected_block_n, completion)
                        .context("Processing close completion")?;
                }

                res = executor.stop.recv(), if !executor_stopped => {
                    executor_stopped = true;
                    if let Err(e) = res.context("In executor thread") {
                        break Err(e);
                    }
                }
            }

            if batcher_completed
                && end_final_block_received
                && self.pending_completions.is_empty()
                && self.pending_canonicalizations.is_empty()
                && self.canonicalization_task.is_none()
                && self.pending_stop_fallback_handoff.is_none()
            {
                tracing::debug!("Shutdown complete: batcher completed, EndFinalBlock processed");
                let shutdown_result = batcher_error
                    .map(|e| {
                        tracing::warn!("Shutdown completed but batcher had error: {e:?}");
                        Err(e)
                    })
                    .unwrap_or(Ok(()));
                break shutdown_result;
            }
        };

        if loop_result.is_err() {
            if let Some(task) = self.canonicalization_task.take() {
                task.abort();
            }
            if let Some(pending) = self.pending_stop_fallback_handoff.take() {
                pending.carry.carry_task.abort();
            }
            if let Some(task) = self.tainted_rebuild_task.take() {
                task.abort();
            }
            self.pending_canonicalizations.clear();
        }

        drop(close_queue_handle);
        let finalizer_result = finalizer_task_handle.join().await;

        while let Some((expected_block_n, rx)) = self.pending_completions.pop_front() {
            match rx.await {
                Ok(Ok(completion)) => {
                    if completion.block_n == expected_block_n {
                        self.diffs_since_snapshot.retain(|(block_n, _)| *block_n > completion.block_n);
                    }
                    if let Some(status) = self.backend.replay_boundary_mark_closed(completion.block_n) {
                        if !status.boundary_met {
                            tracing::warn!(
                                "replay_boundary_closed_without_match block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} mismatch={:?}",
                                status.block_n,
                                status.expected_tx_count,
                                status.executed_tx_count,
                                status.dispatched_tx_count,
                                status.reached_last_tx_hash,
                                status.mismatch
                            );
                        }
                    }
                    if let Err(err) = self.maybe_finish_tainted_rebuild_if_drained() {
                        tracing::warn!(
                            "Shutdown drain: failed to finish tainted rebuild session after close completion for block #{}: {err:#}",
                            completion.block_n
                        );
                    }
                }
                Ok(Err(err)) => {
                    tracing::warn!("Shutdown drain: close completion for block #{} failed: {err:#}", expected_block_n);
                }
                Err(err) => {
                    tracing::warn!("Shutdown drain: completion channel dropped for block #{}: {err}", expected_block_n);
                }
            }
            self.record_block_stage_metrics();
        }

        self.pending_completions.clear();
        self.diffs_since_snapshot.clear();
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: self.backend.latest_confirmed_block_n() });
        self.record_block_stage_metrics();

        match (loop_result, finalizer_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(finalizer_err)) => Err(finalizer_err.context("In finalizer worker")),
            (Err(primary_err), Ok(())) => Err(primary_err),
            (Err(primary_err), Err(finalizer_err)) => {
                tracing::warn!("Finalizer worker also errored during shutdown: {finalizer_err:?}");
                Err(primary_err.context(format!("Additionally, finalizer worker errored: {finalizer_err:#}")))
            }
        }
    }
}
