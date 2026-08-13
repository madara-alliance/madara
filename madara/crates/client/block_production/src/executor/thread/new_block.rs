use super::*;
use crate::executor::ExecutorCommand;

impl ExecutorThread {
    /// current_block_n-10 however might not be saved into the database yet. In that case, we have to wait.
    /// This shouldn't create a deadlock (cyclic wait) unless the database is in a weird state (?)
    ///
    /// https://docs.starknet.io/architecture-and-concepts/network-architecture/starknet-state/#address_0x1
    #[allow(clippy::too_many_arguments)]
    pub(super) fn wait_for_hash_of_block_min_10_or_command(
        &mut self,
        state: &mut ExecutorThreadState,
        pending_routed: &mut RoutedBatchToExecute,
        desired_execution_mode: &mut ExecutionMode,
        execution_epoch: &mut u64,
        wait_for_confirmed_after_fallback: &mut Option<u64>,
        runtime_replay_active: &mut bool,
        replay_current_block_active: &mut bool,
        next_block_deadline: &mut Instant,
        force_close: &mut bool,
        block_empty: &mut bool,
        l2_gas_consumed_block: &mut u128,
        block_time: std::time::Duration,
        block_n: u64,
    ) -> anyhow::Result<WaitForConfirmedHashOutcome> {
        let Some(block_n_min_10) = block_n.checked_sub(10) else {
            return Ok(WaitForConfirmedHashOutcome::Hash(None));
        };

        let backend = Arc::clone(&self.backend);
        let get_hash_from_db = || {
            if let Some(view) = backend.block_view_on_confirmed(block_n_min_10) {
                // block exists
                anyhow::Ok(Some(view.get_block_info().context("Getting block hash of block_n - 10")?.block_hash))
            } else {
                Ok(None)
            }
        };

        // Optimistically get the hash from database without subscribing to the closed_blocks channel.
        if let Some(block_hash) = get_hash_from_db()? {
            Ok(WaitForConfirmedHashOutcome::Hash(Some((block_n_min_10, block_hash))))
        } else {
            tracing::info!(
                "executor_waiting_for_confirmed_hash required_block={} current_block={}",
                block_n_min_10,
                block_n
            );
            let wait_started = std::time::Instant::now();
            loop {
                let mut receiver = self.backend.watch_chain_head_state();
                // We need to re-query the DB here since the it is possible for the block hash to have arrived just in between.
                if let Some(block_hash) = get_hash_from_db()? {
                    tracing::info!(
                        "executor_confirmed_hash_available required_block={} current_block={} wait_ms={}",
                        block_n_min_10,
                        block_n,
                        wait_started.elapsed().as_secs_f64() * 1000.0
                    );
                    break Ok(WaitForConfirmedHashOutcome::Hash(Some((block_n_min_10, block_hash))));
                }
                tracing::debug!(
                    "executor_confirmed_hash_still_pending required_block={} current_block={}",
                    block_n_min_10,
                    block_n
                );
                match self.wait_rt.block_on(async {
                    tokio::select! {
                        Some(cmd) = self.commands.recv() => WaitForConfirmedOutcome::Command(cmd),
                        _ = receiver.changed() => WaitForConfirmedOutcome::Advanced,
                    }
                }) {
                    WaitForConfirmedOutcome::Advanced => {}
                    WaitForConfirmedOutcome::Command(ExecutorCommand::CloseBlock(callback)) => {
                        *force_close = true;
                        let _ = callback.send(Ok(()));
                    }
                    WaitForConfirmedOutcome::Command(ExecutorCommand::ResyncToBackendHead {
                        wait_for_confirmed_block_n,
                    }) => {
                        self.resync_to_backend_head(
                            state,
                            pending_routed,
                            desired_execution_mode,
                            *execution_epoch,
                            wait_for_confirmed_block_n,
                            wait_for_confirmed_after_fallback,
                            runtime_replay_active,
                            replay_current_block_active,
                            next_block_deadline,
                            force_close,
                            block_empty,
                            l2_gas_consumed_block,
                            block_time,
                        )?;
                        break Ok(WaitForConfirmedHashOutcome::ContinueOuterLoop);
                    }
                    WaitForConfirmedOutcome::Command(ExecutorCommand::SetDesiredExecutionMode { mode }) => {
                        self.apply_desired_execution_mode(desired_execution_mode, state, mode);
                    }
                    WaitForConfirmedOutcome::Command(ExecutorCommand::PrepareTaintedRebuildFallback {
                        block_n,
                        execution_epoch: new_epoch,
                        reply,
                    }) => {
                        *desired_execution_mode = ExecutionMode::BlockifierOnly;
                        self.publish_effective_execution_mode(*desired_execution_mode);
                        *execution_epoch = new_epoch;
                        let carry = self.prepare_tainted_rebuild_fallback(
                            state,
                            pending_routed,
                            block_n,
                            *execution_epoch,
                            next_block_deadline,
                            force_close,
                            block_empty,
                            l2_gas_consumed_block,
                            block_time,
                        )?;
                        let _ = reply.send(Ok(carry));
                        *wait_for_confirmed_after_fallback = Some(block_n);
                        *runtime_replay_active = false;
                        *replay_current_block_active = false;
                        self.publish_replay_status(*runtime_replay_active, *execution_epoch);
                        break Ok(WaitForConfirmedHashOutcome::ContinueOuterLoop);
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn sync_new_block_boundary_commands(
        &mut self,
        state: &mut ExecutorThreadState,
        pending_routed: &mut RoutedBatchToExecute,
        desired_execution_mode: &mut ExecutionMode,
        execution_epoch: &mut u64,
        wait_for_confirmed_after_fallback: &mut Option<u64>,
        runtime_replay_active: &mut bool,
        replay_current_block_active: &mut bool,
        next_block_deadline: &mut Instant,
        force_close: &mut bool,
        block_empty: &mut bool,
        l2_gas_consumed_block: &mut u128,
        block_time: std::time::Duration,
    ) -> anyhow::Result<NewBlockBoundarySyncOutcome> {
        while matches!(state, ExecutorThreadState::NewBlock(_)) {
            let Ok(cmd) = self.commands.try_recv() else { break };
            match cmd {
                ExecutorCommand::CloseBlock(callback) => {
                    *force_close = true;
                    let _ = callback.send(Ok(()));
                }
                ExecutorCommand::ResyncToBackendHead { wait_for_confirmed_block_n } => {
                    self.resync_to_backend_head(
                        state,
                        pending_routed,
                        desired_execution_mode,
                        *execution_epoch,
                        wait_for_confirmed_block_n,
                        wait_for_confirmed_after_fallback,
                        runtime_replay_active,
                        replay_current_block_active,
                        next_block_deadline,
                        force_close,
                        block_empty,
                        l2_gas_consumed_block,
                        block_time,
                    )?;
                    return Ok(NewBlockBoundarySyncOutcome::ContinueOuterLoop);
                }
                ExecutorCommand::SetDesiredExecutionMode { mode } => {
                    self.apply_desired_execution_mode(desired_execution_mode, state, mode);
                }
                ExecutorCommand::PrepareTaintedRebuildFallback { block_n, execution_epoch: new_epoch, reply } => {
                    *desired_execution_mode = ExecutionMode::BlockifierOnly;
                    self.publish_effective_execution_mode(*desired_execution_mode);
                    *execution_epoch = new_epoch;
                    let carry = self.prepare_tainted_rebuild_fallback(
                        state,
                        pending_routed,
                        block_n,
                        *execution_epoch,
                        next_block_deadline,
                        force_close,
                        block_empty,
                        l2_gas_consumed_block,
                        block_time,
                    )?;
                    let _ = reply.send(Ok(carry));
                    *wait_for_confirmed_after_fallback = Some(block_n);
                    *runtime_replay_active = false;
                    *replay_current_block_active = false;
                    self.publish_replay_status(*runtime_replay_active, *execution_epoch);
                    return Ok(NewBlockBoundarySyncOutcome::ContinueOuterLoop);
                }
            }
        }

        Ok(NewBlockBoundarySyncOutcome::Proceed)
    }
}
