use super::*;

impl ExecutorThread {
    pub(super) fn fallback_future_routed_block_n(
        state: &ExecutorThreadState,
        pending_routed: &RoutedBatchToExecute,
        fallback_block_n: u64,
    ) -> u64 {
        if !pending_routed.is_empty() {
            return pending_routed.block_n.saturating_add(1);
        }

        match state {
            ExecutorThreadState::Executing(s) if s.exec_ctx.block_number > fallback_block_n => {
                s.exec_ctx.block_number.saturating_add(1)
            }
            ExecutorThreadState::NewBlock(s) if s.state_adaptor.block_n() > fallback_block_n => {
                s.state_adaptor.block_n()
            }
            _ => fallback_block_n.saturating_add(1),
        }
    }

    pub(super) fn normalize_routed_batch_for_execution_mode(
        routed: &mut RoutedBatchToExecute,
        target_execution_mode: ExecutionMode,
    ) {
        if routed.execution_mode == target_execution_mode {
            return;
        }

        if target_execution_mode == ExecutionMode::BlockifierOnly && !routed.rust_batch.is_empty() {
            routed.blockifier_batch.extend(mem::take(&mut routed.rust_batch));
        }

        routed.execution_mode = target_execution_mode;
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn resync_to_backend_head(
        &mut self,
        state: &mut ExecutorThreadState,
        pending_routed: &mut RoutedBatchToExecute,
        desired_execution_mode: &mut ExecutionMode,
        execution_epoch: u64,
        runtime_replay_active: &mut bool,
        replay_current_block_active: &mut bool,
        next_block_deadline: &mut Instant,
        force_close: &mut bool,
        block_empty: &mut bool,
        l2_gas_consumed_block: &mut u128,
        block_time: std::time::Duration,
    ) -> anyhow::Result<()> {
        let stale_block_n = match state {
            ExecutorThreadState::Executing(s) => Some(s.exec_ctx.block_number),
            ExecutorThreadState::NewBlock(s) => Some(s.state_adaptor.block_n()),
        };
        let backend_head = self.backend.chain_head_state();
        let dropped_pending_txs = pending_routed.total_len();

        *state = self.initial_state().context("Resyncing executor state to backend head")?;
        *pending_routed = RoutedBatchToExecute::default();
        *desired_execution_mode = *self.execution_mode_rx.borrow();
        *runtime_replay_active = false;
        *replay_current_block_active = false;
        *next_block_deadline = Instant::now() + block_time;
        *force_close = false;
        *block_empty = true;
        *l2_gas_consumed_block = 0;
        self.publish_effective_execution_mode(*desired_execution_mode);
        self.publish_replay_status(*runtime_replay_active, execution_epoch);

        let next_block_n = match state {
            ExecutorThreadState::NewBlock(s) => s.state_adaptor.block_n(),
            ExecutorThreadState::Executing(_) => unreachable!("resync must reset executor to NewBlock"),
        };
        tracing::info!(
            target: "RUST_EXEC",
            stale_block_n = ?stale_block_n,
            dropped_pending_txs,
            backend_confirmed_tip = ?backend_head.confirmed_tip,
            backend_internal_tip = ?backend_head.internal_preconfirmed_tip,
            next_block_n,
            desired_execution_mode = ?*desired_execution_mode,
            execution_epoch,
            "Executor resynced to backend head"
        );

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn resume_after_tainted_rebuild(
        &mut self,
        state: &mut ExecutorThreadState,
        pending_routed: &mut RoutedBatchToExecute,
        desired_execution_mode: &mut ExecutionMode,
        execution_epoch: u64,
        expected_confirmed_head: u64,
        runtime_replay_active: &mut bool,
        replay_current_block_active: &mut bool,
        next_block_deadline: &mut Instant,
        force_close: &mut bool,
        block_empty: &mut bool,
        l2_gas_consumed_block: &mut u128,
        block_time: std::time::Duration,
    ) -> Result<crate::executor::TaintedRebuildResumeAck, crate::executor::ExecutorCommandError> {
        let confirmed_head = self.backend.latest_confirmed_block_n().ok_or_else(|| {
            crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(
                "backend has no confirmed head".to_owned(),
            )
        })?;
        if confirmed_head != expected_confirmed_head {
            return Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(format!(
                "expected confirmed head #{expected_confirmed_head}, found #{confirmed_head}"
            )));
        }
        if *self.execution_mode_rx.borrow() != ExecutionMode::BlockifierOnly {
            return Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(format!(
                "expected BlockifierOnly mode, found {:?}",
                *self.execution_mode_rx.borrow()
            )));
        }

        self.resync_to_backend_head(
            state,
            pending_routed,
            desired_execution_mode,
            execution_epoch,
            runtime_replay_active,
            replay_current_block_active,
            next_block_deadline,
            force_close,
            block_empty,
            l2_gas_consumed_block,
            block_time,
        )
        .map_err(|err| crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(format!("{err:#}")))?;

        let next_block_n = match state {
            ExecutorThreadState::NewBlock(state) => state.state_adaptor.block_n(),
            ExecutorThreadState::Executing(_) => unreachable!("resync must leave executor at a new-block boundary"),
        };
        if next_block_n != confirmed_head.saturating_add(1) {
            return Err(crate::executor::ExecutorCommandError::InvalidTaintedRebuildResume(format!(
                "expected next block #{}, found #{next_block_n}",
                confirmed_head.saturating_add(1)
            )));
        }

        Ok(crate::executor::TaintedRebuildResumeAck { confirmed_head, next_block_n, execution_epoch })
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_tainted_rebuild_fallback(
        &mut self,
        state: &mut ExecutorThreadState,
        pending_routed: &mut RoutedBatchToExecute,
        fallback_block_n: u64,
        execution_epoch: u64,
        next_block_deadline: &mut Instant,
        force_close: &mut bool,
        block_empty: &mut bool,
        l2_gas_consumed_block: &mut u128,
        block_time: std::time::Duration,
    ) -> anyhow::Result<Vec<TaintedRebuildCarryTx>> {
        let mut carry = Vec::new();
        let mut discarded_block_n = None;
        match state {
            ExecutorThreadState::Executing(s) if s.exec_ctx.block_number > fallback_block_n => {
                discarded_block_n = Some(s.exec_ctx.block_number);
                extend_carry_txs(&mut carry, mem::take(&mut s.executed_in_block), discarded_block_n);
                *state = self.initial_state().context("Resetting executor state after fallback")?;
                *next_block_deadline = Instant::now() + block_time;
                *force_close = false;
                *block_empty = true;
                *l2_gas_consumed_block = 0;
            }
            ExecutorThreadState::NewBlock(s) if s.state_adaptor.block_n() > fallback_block_n => {
                discarded_block_n = Some(s.state_adaptor.block_n());
                *state = self.initial_state().context("Resetting executor new-block frontier after fallback")?;
                *next_block_deadline = Instant::now() + block_time;
                *force_close = false;
                *block_empty = true;
                *l2_gas_consumed_block = 0;
            }
            _ => {}
        }

        let queued_future_block_n = Some(Self::fallback_future_routed_block_n(state, pending_routed, fallback_block_n));

        // Append executor-local pending txs last (not yet executed).
        let pending_block_n = Some(pending_routed.block_n);
        extend_carry_txs(&mut carry, mem::take(&mut pending_routed.blockifier_batch), pending_block_n);
        extend_carry_txs(&mut carry, mem::take(&mut pending_routed.rust_batch), pending_block_n);

        // C-021: Also absorb any already-routed batches sitting in the batcher->executor
        // channel. These txs have already left the mempool, but were not yet reflected in
        // executor local state or canonicalization state. On fallback they must become part
        // of the replay payload, not get lost at the handoff seam.
        let mut drained_routed_batches = 0usize;
        let mut drained_routed_txs = 0usize;
        while let Ok(stale_routed) = self.incoming_batches.try_recv() {
            drained_routed_batches += 1;
            drained_routed_txs += stale_routed.total_len();
            let stale_block_n = queued_future_block_n;
            extend_carry_txs(&mut carry, stale_routed.blockifier_batch, stale_block_n);
            extend_carry_txs(&mut carry, stale_routed.rust_batch, stale_block_n);
        }

        let pre_dedup = carry.len();
        let mut seen_hashes = HashSet::new();
        let carry: Vec<TaintedRebuildCarryTx> =
            carry.into_iter().filter(|carry_tx| seen_hashes.insert(carry_tx.tx.tx_hash().to_felt())).collect();
        let n_deduped = pre_dedup - carry.len();
        pending_routed.blockifier_batch = BatchToExecute::default();
        pending_routed.rust_batch = BatchToExecute::default();
        pending_routed.block_n = 0;
        pending_routed.execution_mode = ExecutionMode::BlockifierOnly;
        let replay_summary = summarize_carry_txs(&carry);

        tracing::info!(
            fallback_block_n,
            discarded_block_n = ?discarded_block_n,
            carry_txs = carry.len(),
            n_deduped,
            drained_routed_batches,
            drained_routed_txs,
            replay_first_tx_hash = replay_summary.first_hash.as_deref().unwrap_or("-"),
            replay_first_tx_nonce = replay_summary.first_nonce.as_deref().unwrap_or("-"),
            replay_last_tx_hash = replay_summary.last_hash.as_deref().unwrap_or("-"),
            replay_last_tx_nonce = replay_summary.last_nonce.as_deref().unwrap_or("-"),
            execution_epoch,
            "executor_tainted_rebuild_fallback_prepared"
        );

        Ok(carry)
    }

    pub(super) fn replay_boundary_exists(&self, block_n: u64) -> bool {
        self.backend.replay_boundary_exists(block_n)
    }

    pub(super) fn replay_boundary_remaining_capacity(&self, block_n: u64) -> Option<usize> {
        self.backend
            .replay_boundary_remaining_execution_capacity(block_n)
            .map(|remaining| usize::try_from(remaining).unwrap_or(usize::MAX))
    }

    pub(super) fn replay_boundary_is_met(&self, block_n: u64) -> Option<bool> {
        self.backend.replay_boundary_is_met(block_n)
    }

    pub(super) fn record_replay_executed_hashes(&self, block_n: u64, replay_executed_hashes: &[Felt]) {
        if !self.replay_mode_enabled || replay_executed_hashes.is_empty() {
            return;
        }

        if let Some(status) = self.backend.replay_boundary_record_executed_hashes(block_n, replay_executed_hashes) {
            if let Some(mismatch) = status.mismatch {
                tracing::warn!(
                    "replay_boundary_mismatch_after_execution block_number={} expected_tx_count={} executed_tx_count={} dispatched_tx_count={} reached_last_tx_hash={} message={}",
                    block_n,
                    status.expected_tx_count,
                    status.executed_tx_count,
                    status.dispatched_tx_count,
                    status.reached_last_tx_hash,
                    mismatch
                );
            }
        }
    }
}
