use super::{CloseDecision, CloseReason, ExecutorThread, ExecutorThreadState};
use crate::util::BatchToExecute;
use mp_convert::Felt;
use tokio::time::Instant;

impl ExecutorThread {
    pub(super) fn replay_boundary_exists(&self, block_n: u64) -> bool {
        self.backend.replay_boundary_exists(block_n)
    }

    fn replay_boundary_remaining_capacity(&self, block_n: u64) -> Option<usize> {
        self.backend
            .replay_boundary_remaining_execution_capacity(block_n)
            .map(|remaining| usize::try_from(remaining).unwrap_or(usize::MAX))
    }

    fn replay_boundary_is_met(&self, block_n: u64) -> bool {
        self.backend.replay_boundary_is_met(block_n).unwrap_or(false)
    }

    /// Replay boundaries suspend the usual block-time deadline while the executor
    /// is still working through the replay-capped block.
    pub(super) fn replay_wait_deadline(
        &self,
        state: &ExecutorThreadState,
        block_empty: bool,
        no_empty_blocks: bool,
        next_block_deadline: Instant,
    ) -> Option<Instant> {
        let replay_boundary_active = self.replay_mode_enabled
            && matches!(
                state,
                ExecutorThreadState::Executing(executing_state)
                    if self.replay_boundary_exists(executing_state.exec_ctx.block_number)
            );

        if replay_boundary_active || (block_empty && no_empty_blocks) {
            None
        } else {
            Some(next_block_deadline)
        }
    }

    /// Replay boundaries cap how many transactions the current block may execute.
    /// Excess transactions stay buffered and roll into the next block unchanged.
    pub(super) fn apply_replay_boundary_capacity(
        &self,
        block_n: u64,
        to_exec: &mut BatchToExecute,
        replay_next_block_buffer: &mut BatchToExecute,
    ) {
        if !self.replay_mode_enabled {
            return;
        }

        let Some(remaining) = self.replay_boundary_remaining_capacity(block_n) else {
            return;
        };

        if to_exec.len() <= remaining {
            return;
        }

        let overflow_txs = to_exec.txs.split_off(remaining);
        let overflow_additional_info = to_exec.additional_info.split_off(remaining);
        let overflow_count = overflow_txs.len();
        replay_next_block_buffer
            .extend(BatchToExecute { txs: overflow_txs, additional_info: overflow_additional_info });
        self.metrics.replay_boundary_spillover_transactions_total.add(overflow_count as u64, &[]);
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

    /// Replay boundaries override the normal block-full / block-time close rules.
    /// A replay-capped block closes only when the boundary is met or a force-close
    /// command arrives.
    pub(super) fn replay_close_decision(
        &self,
        block_n: u64,
        force_close: bool,
        block_full: bool,
        block_time_deadline_reached: bool,
    ) -> CloseDecision {
        let replay_boundary_exists = self.replay_mode_enabled && self.replay_boundary_exists(block_n);
        let replay_boundary_met = replay_boundary_exists && self.replay_boundary_is_met(block_n);

        let reason = if force_close {
            Some(CloseReason::ForceClose)
        } else if replay_boundary_exists {
            replay_boundary_met.then_some(CloseReason::ReplayBoundaryMet)
        } else if block_full {
            Some(CloseReason::BlockFull)
        } else if block_time_deadline_reached {
            Some(CloseReason::BlockTimeDeadline)
        } else {
            None
        };

        CloseDecision { should_close: reason.is_some(), replay_boundary_exists, replay_boundary_met, reason }
    }
}
