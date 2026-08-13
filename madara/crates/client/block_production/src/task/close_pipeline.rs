use super::*;

impl BlockProductionTask {
    /// Close and save a block using the execution summary.
    /// Used for both normal block closing (EndBlock) and shutdown (EndFinalBlock).
    pub(super) async fn close_block(
        &mut self,
        block_exec_summary: Box<BlockExecutionSummary>,
        _close_queue: &FinalizerHandle,
    ) -> anyhow::Result<()> {
        let current_state = self.current_state.take().context("No current state")?;
        let TaskState::Executing(state) = current_state else {
            anyhow::bail!("Invalid executor state transition: expected current state to be Executing")
        };
        let block_n = state.block_number;
        let execution_mode = state.execution_snapshot.execution_mode;
        let tx_count = state.accumulated_stats.n_added_to_block;
        tracing::debug!("close_block_received_from_executor block_number={block_n}");
        // C-018: Do not compute parent_overlays here. They are recomputed at
        // canonicalization start to avoid stale overlays from parent stops.
        self.pending_canonicalizations.push_back(PendingCanonicalizationInput { state, block_exec_summary });
        self.current_state = Some(TaskState::NotExecuting { latest_block_n: Some(block_n) });
        if execution_mode == ExecutionMode::Mixed {
            let window = self.internal_preconfirmed_window();
            tracing::info!(
                target: "RUST_EXEC",
                block_number = block_n,
                tx_count,
                confirmed_tip = ?window.confirmed_tip,
                external_preconfirmed_tip = ?window.external_preconfirmed_tip,
                internal_preconfirmed_tip = ?window.internal_preconfirmed_tip,
                internal_depth = window.depth,
                internal_capacity = window.capacity,
                "📥 Block #{} added to internal preconfirmed window at {}/{} (confirmed tip {:?})",
                block_n,
                window.depth,
                window.capacity,
                window.confirmed_tip,
            );
        }
        self.record_block_stage_metrics();
        self.maybe_start_canonicalization_task();
        Ok(())
    }

    pub(super) async fn enqueue_canonical_close_payload(
        &mut self,
        close_queue: &FinalizerHandle,
        state: CurrentBlockState,
        canonical_bouncer_weights: blockifier::bouncer::BouncerWeights,
        canonical_state_diff: StateDiff,
        canonical_rows_for_close: Vec<PreconfirmedExecutedTransaction>,
        canonical_header_for_close: PreconfirmedHeader,
    ) -> anyhow::Result<()> {
        let block_n = state.block_number;
        self.diffs_since_snapshot.push((block_n, canonical_state_diff.clone()));

        let payload = QueuedClosePayload {
            db_payload: mc_db::close_pipeline_contract::CloseJobPayload { block_n },
            state,
            canonical_bouncer_weights,
            state_diff: canonical_state_diff,
            canonical_executed_rows: canonical_rows_for_close,
            canonical_header: canonical_header_for_close,
            internal_capacity: self.close_queue_capacity,
            enqueued_at: Instant::now(),
        };
        tracing::debug!("enqueue_close_block_to_async_worker block_number={block_n}");
        let (queued_result, completion) = close_queue.enqueue(payload).await?;
        let ClosePreconfirmedResult::Queued(queued_meta) = queued_result;
        let queue_depth = close_queue.current_depth();
        let queue_in_flight = close_queue.current_in_flight();
        let pending_close_completions = self.pending_completions.len() + 1;
        self.metrics.close_queue_enqueued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(queue_depth as u64, &[]);
        tracing::debug!(
            "close_block_queued block_number={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={}",
            queued_meta.block_n,
            queue_depth,
            close_queue.configured_capacity(),
            queue_in_flight,
            pending_close_completions
        );
        self.pending_completions.push_back((block_n, completion));
        self.record_block_stage_metrics();
        Ok(())
    }

    pub(super) fn handle_close_completion(
        &mut self,
        close_queue: &FinalizerHandle,
        expected_block_n: u64,
        completion: CloseJobCompletion,
    ) -> anyhow::Result<()> {
        self.metrics.close_queue_dequeued_total.add(1, &[]);
        self.metrics.close_queue_depth.record(close_queue.current_depth() as u64, &[]);
        tracing::debug!(
            "close_block_complete block_number={} expected_block_n={} queue_depth={} queue_capacity={} queue_in_flight={} pending_close_completions={}",
            completion.block_n,
            expected_block_n,
            close_queue.current_depth(),
            close_queue.configured_capacity(),
            close_queue.current_in_flight(),
            self.pending_completions.len()
        );
        if completion.block_n != expected_block_n {
            anyhow::bail!("Out-of-order close completion: expected #{expected_block_n}, got #{}", completion.block_n);
        }
        self.diffs_since_snapshot.retain(|(block_n, _)| *block_n > completion.block_n);
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
        self.try_publish_current_external_shell()
            .context("Publishing comparator-approved external shell after close completion")?;
        self.send_state_notification(BlockProductionStateNotification::ClosedBlock { block_n: completion.block_n });
        let drained = self
            .maybe_finish_tainted_rebuild_if_drained()
            .context("Finishing drained tainted rebuild session after close completion")?;
        if !drained {
            if let Some(session) = self.tainted_rebuild_session.as_ref() {
                self.queue_post_close_executor_resync(Some(session.next_block_n), "active tainted rebuild close")
                    .context("Queueing executor resync toward the next tainted rebuild block")?;
            }
        }
        self.record_block_stage_metrics();
        Ok(())
    }

    #[allow(dead_code)]
    pub(super) async fn execute_close_payload(
        metrics: Arc<BlockProductionMetrics>,
        payload: QueuedClosePayload,
    ) -> anyhow::Result<CloseJobCompletion> {
        let QueuedClosePayload {
            state,
            canonical_bouncer_weights,
            state_diff,
            canonical_executed_rows,
            canonical_header,
            internal_capacity,
            ..
        } = payload;
        tracing::debug!("Close and save block block_n={}", state.block_number);
        let start_time = Instant::now();

        // C-024: Derive tx count and event count from canonical rows carried in the
        // close payload. Do NOT re-read from block_view_on_preconfirmed — under async
        // runahead, internal runtime may be at X+N and DB preconfirmed may be stale.
        let n_txs = canonical_executed_rows.len();
        let event_count =
            canonical_executed_rows.iter().map(|tx| tx.transaction.receipt.events().len() as u64).sum::<u64>();
        let declared_classes_count = state_diff.declared_classes.len();
        let deployed_contracts_count = state_diff.deployed_contracts.len();
        let storage_diffs_count = state_diff.storage_diffs.len();
        let nonce_updates_count = state_diff.nonces.len();
        let state_diff_len = state_diff.len();
        let consumed_l1_nonces_count = state.consumed_core_contract_nonces.len();
        let confirmed_tip_before = state.backend.chain_head_state().confirmed_tip;

        let bouncer_l1_gas = canonical_bouncer_weights.l1_gas;
        let bouncer_sierra_gas = canonical_bouncer_weights.sierra_gas.0;
        let bouncer_n_events = canonical_bouncer_weights.n_events;
        let bouncer_message_segment_length = canonical_bouncer_weights.message_segment_length;
        let bouncer_state_diff_size = canonical_bouncer_weights.state_diff_size;

        metrics.block_declared_classes_count.record(declared_classes_count as u64, &[]);
        metrics.block_deployed_contracts_count.record(deployed_contracts_count as u64, &[]);
        metrics.block_storage_diffs_count.record(storage_diffs_count as u64, &[]);
        metrics.block_nonce_updates_count.record(nonce_updates_count as u64, &[]);
        metrics.block_state_diff_length.record(state_diff_len as u64, &[]);
        metrics.block_event_count.record(event_count, &[]);

        metrics.block_bouncer_l1_gas.record(bouncer_l1_gas as u64, &[]);
        metrics.block_bouncer_sierra_gas.record(bouncer_sierra_gas, &[]);
        metrics.block_bouncer_n_events.record(bouncer_n_events as u64, &[]);
        metrics.block_bouncer_message_segment_length.record(bouncer_message_segment_length as u64, &[]);
        metrics.block_bouncer_state_diff_size.record(bouncer_state_diff_size as u64, &[]);
        metrics.block_consumed_l1_nonces_count.record(consumed_l1_nonces_count as u64, &[]);

        let close_preconfirmed_start = Instant::now();
        let db_result = Self::close_canonical_block_with_state_diff(
            state.backend.clone(),
            state.block_number,
            state.consumed_core_contract_nonces,
            &canonical_bouncer_weights,
            state_diff,
            canonical_header,
            canonical_executed_rows,
        )
        .await
        .context("Closing block")?;
        let close_preconfirmed_duration = close_preconfirmed_start.elapsed();
        let window_after = InternalPreconfirmedWindowSnapshot::from_backend(&state.backend, internal_capacity);
        let slots_freed = window_after.confirmed_advance_from(confirmed_tip_before);
        let depth_before_close = window_after.depth.saturating_add(slots_freed).min(window_after.capacity);
        metrics.close_preconfirmed_duration.record(close_preconfirmed_duration.as_secs_f64(), &[]);
        metrics.close_preconfirmed_last.record(close_preconfirmed_duration.as_secs_f64(), &[]);

        let time_to_close = start_time.elapsed();
        let block_production_time = state.block_start_time.elapsed();

        let timings = &db_result.timings;
        let exec_stats = &state.accumulated_stats;
        tracing::info!(
            target: "close_block",
            block_number = state.block_number,
            tx_count = n_txs,
            event_count = event_count,
            close_block_total_ms = time_to_close.as_secs_f64() * 1000.0,
            block_close_ms = time_to_close.as_secs_f64() * 1000.0,
            close_preconfirmed_ms = close_preconfirmed_duration.as_secs_f64() * 1000.0,
            block_production_ms = block_production_time.as_secs_f64() * 1000.0,
            batches_executed = exec_stats.n_batches,
            txs_added_to_block = exec_stats.n_added_to_block,
            txs_executed = exec_stats.n_executed,
            txs_reverted = exec_stats.n_reverted,
            txs_rejected = exec_stats.n_rejected,
            classes_declared = exec_stats.declared_classes,
            l2_gas_consumed = exec_stats.l2_gas_consumed,
            state_diff_len = state_diff_len,
            declared_classes = declared_classes_count,
            deployed_contracts = deployed_contracts_count,
            storage_diffs = storage_diffs_count,
            nonce_updates = nonce_updates_count,
            consumed_l1_nonces = consumed_l1_nonces_count,
            bouncer_l1_gas = bouncer_l1_gas,
            bouncer_sierra_gas = bouncer_sierra_gas,
            bouncer_n_events = bouncer_n_events,
            bouncer_message_segment_length = bouncer_message_segment_length,
            bouncer_state_diff_size = bouncer_state_diff_size,
            get_full_block_ms = timings.get_full_block_with_classes.as_secs_f64() * 1000.0,
            commitments_ms = timings.block_commitments_compute.as_secs_f64() * 1000.0,
            merklization_ms = timings.merklization.as_secs_f64() * 1000.0,
            contract_trie_ms = timings.contract_trie_root.as_secs_f64() * 1000.0,
            class_trie_ms = timings.class_trie_root.as_secs_f64() * 1000.0,
            contract_storage_trie_commit_ms = timings.contract_storage_trie_commit.as_secs_f64() * 1000.0,
            contract_trie_commit_ms = timings.contract_trie_commit.as_secs_f64() * 1000.0,
            class_trie_commit_ms = timings.class_trie_commit.as_secs_f64() * 1000.0,
            block_hash_ms = timings.block_hash_compute.as_secs_f64() * 1000.0,
            db_write_ms = timings.db_write_block_parts.as_secs_f64() * 1000.0,
            confirmed_tip = ?window_after.confirmed_tip,
            external_preconfirmed_tip = ?window_after.external_preconfirmed_tip,
            internal_preconfirmed_tip = ?window_after.internal_preconfirmed_tip,
            internal_depth = window_after.depth,
            internal_capacity = window_after.capacity,
            internal_slots_freed = slots_freed,
            "block_closed"
        );

        if state.execution_snapshot.execution_mode == ExecutionMode::Mixed {
            tracing::info!(
                "⛏️  Closed block #{} with {n_txs} transactions in {time_to_close:?} | RUST_EXEC internal preconfirmed window {} -> {}/{} (freed {} slot(s))",
                state.block_number,
                depth_before_close,
                window_after.depth,
                window_after.capacity,
                slots_freed,
            );
        } else {
            tracing::info!("⛏️  Closed block #{} with {n_txs} transactions in {time_to_close:?}", state.block_number);
        }

        metrics.close_block_total_duration.record(time_to_close.as_secs_f64(), &[]);
        metrics.close_block_total_last.record(time_to_close.as_secs_f64(), &[]);
        metrics.block_counter.add(1, &[]);
        metrics.block_gauge.record(state.block_number, &[]);
        metrics.transaction_counter.add(n_txs as u64, &[]);
        metrics.block_production_time.record(block_production_time.as_secs_f64(), &[]);
        metrics.block_production_time_last.record(block_production_time.as_secs_f64(), &[]);
        metrics.block_close_time.record(time_to_close.as_secs_f64(), &[]);
        metrics.block_close_time_last.record(time_to_close.as_secs_f64(), &[]);

        Ok(CloseJobCompletion { block_n: state.block_number })
    }
}
