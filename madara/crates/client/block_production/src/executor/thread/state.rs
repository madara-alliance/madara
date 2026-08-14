use super::*;

impl ExecutorThread {
    pub(super) fn end_block(&mut self, state: &mut ExecutorStateExecuting) -> anyhow::Result<ExecutorThreadState> {
        let mut cached_state = state.executor.block_state.take().expect("Executor block state already taken");

        let block_number = state.exec_ctx.block_number;
        let state_diff = cached_state.to_state_diff().context("Cannot make state diff")?.state_maps;
        if mc_rust_exec::config::tx_diff_log_enabled()
            || mc_rust_exec::config::debug_block().is_some_and(|debug_block| debug_block == block_number)
        {
            let zero_entries =
                state_diff.storage_updates.values().flatten().filter(|(_, value)| **value == Felt::ZERO).count();
            tracing::info!(
                target: "RUST_EXEC",
                "final_block_state_diff block_number={} stage=finalized storage_entries={} storage_contracts={} nonce_updates={} zero_entries={}",
                block_number,
                state_diff.storage_updates.values().map(|updates| updates.len()).sum::<usize>(),
                state_diff.storage_updates.len(),
                state_diff.address_to_nonce.len(),
                zero_entries,
            );
            for (contract_address, updates) in &state_diff.storage_updates {
                for (storage_key, value) in updates {
                    if *value == Felt::ZERO {
                        tracing::info!(
                            target: "RUST_EXEC",
                            "final_block_state_diff_entry block_number={} contract_address={:#x} storage_key={:#x} value=0x0",
                            block_number,
                            contract_address.to_felt(),
                            storage_key.to_felt(),
                        );
                    }
                }
            }
        }
        let mut cached_adapter = cached_state.state;
        cached_adapter.finish_block(
            state_diff,
            mem::take(&mut state.declared_classes),
            mem::take(&mut state.consumed_l1_to_l2_nonces),
        )?;

        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: cached_adapter,
            consumed_l1_to_l2_nonces: HashSet::new(),
        }))
    }

    /// Returns the initial state diff storage too. It is used to create the StartNewBlock message and transition to ExecutorState::Executing.
    pub(super) fn create_execution_state(
        &mut self,
        state: ExecutorStateNewBlock,
        previous_l2_gas_used: u128,
        execution_mode: ExecutionMode,
        block_n_min_10_hash: Option<(u64, Felt)>,
    ) -> anyhow::Result<ExecutorStateExecuting> {
        let previous_l2_gas_price = state.state_adaptor.latest_gas_prices().strk_l2_gas_price;
        let exec_ctx = create_execution_context(
            &self.backend,
            state.state_adaptor.block_n(),
            previous_l2_gas_price,
            previous_l2_gas_used,
        )?;

        // Create the TransactionExecutor with block_n-10 handling, reusing the layered_state_adapter.
        let executor = crate::util::create_executor_with_block_n_min_10(
            &self.backend,
            &exec_ctx,
            state.state_adaptor,
            move |_block_n| Ok(block_n_min_10_hash),
            None, // Use backend's chain_config (normal execution)
        )?;

        Ok(ExecutorStateExecuting {
            exec_ctx,
            execution_mode,
            executor,
            consumed_l1_to_l2_nonces: state.consumed_l1_to_l2_nonces,
            declared_classes: HashMap::new(),
            rust_phase_state: RustPhaseState { first_tx_in_block: true, projected_bouncer_weights: None },
            executed_in_block: BatchToExecute::default(),
        })
    }

    pub(super) fn initial_state(&self) -> anyhow::Result<ExecutorThreadState> {
        Ok(ExecutorThreadState::NewBlock(ExecutorStateNewBlock {
            state_adaptor: LayeredStateAdapter::new(Arc::clone(&self.backend))?,
            consumed_l1_to_l2_nonces: HashSet::new(),
        }))
    }

    pub(super) fn publish_effective_execution_mode(&self, execution_mode: ExecutionMode) {
        let _ = self.execution_mode_tx.send(execution_mode);
    }

    pub(super) fn publish_replay_status(&self, replay_active: bool, execution_epoch: u64) {
        let replay_status = if replay_active {
            RuntimeReplayStatus::in_progress_for_epoch(execution_epoch)
        } else {
            RuntimeReplayStatus::idle_for_epoch(execution_epoch)
        };
        let _ = self.replay_status_tx.send(replay_status);
    }

    pub(super) fn apply_desired_execution_mode(
        &mut self,
        desired_execution_mode: &mut ExecutionMode,
        state: &ExecutorThreadState,
        mode: ExecutionMode,
    ) {
        *desired_execution_mode = mode;
        if matches!(state, ExecutorThreadState::NewBlock(_)) {
            self.publish_effective_execution_mode(mode);
        }
    }

    pub(super) fn current_executor_block_n(state: &ExecutorThreadState) -> u64 {
        match state {
            ExecutorThreadState::Executing(s) => s.exec_ctx.block_number,
            ExecutorThreadState::NewBlock(s) => s.state_adaptor.block_n(),
        }
    }

    pub(super) fn rebind_routed_batch_to_block(routed: &mut RoutedBatchToExecute, block_n: u64) {
        if !routed.is_empty() {
            routed.block_n = block_n;
        }
    }
}
