use super::types::{ExecutionMode, ExecutionboxStatus, FallbackReason, RuntimeReplayStatus, StartupExecutionMode};

pub struct FallbackManager {
    pub mode: ExecutionMode,
    pub startup_mode: StartupExecutionMode,
    pub startup_recovery_active: bool,
    pub reason: Option<FallbackReason>,
    pub taint_block: Option<u64>,
    pub comparator_enabled: bool,
    pub reexec_epoch: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableOutcome {
    EnabledNow,
    AlreadyMixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnableError {
    ReplayInProgress,
}

impl FallbackManager {
    pub fn new(startup_mode: StartupExecutionMode) -> Self {
        Self {
            mode: ExecutionMode::BlockifierOnly,
            startup_mode,
            startup_recovery_active: true,
            reason: None,
            taint_block: None,
            comparator_enabled: false,
            reexec_epoch: 0,
        }
    }

    /// Synchronous enable decision. No intent latch in V1.
    ///
    /// - startup recovery active -> ReplayInProgress (no state change)
    /// - replay inactive + BlockifierOnly -> switch to Mixed immediately
    /// - already Mixed -> idempotent success
    pub fn executionbox_enable(&mut self) -> Result<EnableOutcome, EnableError> {
        if self.startup_recovery_active {
            return Err(EnableError::ReplayInProgress);
        }

        if self.mode == ExecutionMode::Mixed {
            return Ok(EnableOutcome::AlreadyMixed);
        }

        self.mode = ExecutionMode::Mixed;
        self.reason = None;
        self.taint_block = None;
        self.comparator_enabled = true;

        Ok(EnableOutcome::EnabledNow)
    }

    /// Force disable ExecutionBox. Idempotent if already disabled.
    pub fn executionbox_disable(&mut self) {
        if self.mode == ExecutionMode::BlockifierOnly {
            return;
        }

        self.mode = ExecutionMode::BlockifierOnly;
        self.reason = Some(FallbackReason::ManualForceDisable);
        self.comparator_enabled = false;
    }

    /// Enter fallback mode at anchor block X.
    ///
    /// Replay ownership is external to the manager. The executor owns runtime replay
    /// backlog / drain truth; the manager only tracks policy/mode state.
    pub fn enter_fallback(&mut self, reason: FallbackReason, anchor_block_x: u64) {
        self.mode = ExecutionMode::BlockifierOnly;
        self.reason = Some(reason);
        self.taint_block = Some(anchor_block_x);
        self.comparator_enabled = false;
        self.reexec_epoch += 1;
    }

    /// Called when startup preconfirmed recovery completes.
    ///
    /// Maps startup config to runtime mode:
    /// - startup_mode Mixed -> set mode to Mixed, enable comparator
    /// - startup_mode BlockifierOnly -> keep BlockifierOnly, comparator disabled
    pub fn on_startup_recovery_complete(&mut self) {
        self.startup_recovery_active = false;

        match self.startup_mode {
            StartupExecutionMode::Mixed => {
                self.mode = ExecutionMode::Mixed;
                self.reason = None;
                self.taint_block = None;
                self.comparator_enabled = true;
            }
            StartupExecutionMode::BlockifierOnly => {
                self.mode = ExecutionMode::BlockifierOnly;
                self.comparator_enabled = false;
            }
        }
    }

    /// Finish restart recovery without undoing a previously entered persistent fallback.
    pub fn on_persistent_fallback_recovery_complete(&mut self) {
        self.startup_recovery_active = false;
        self.mode = ExecutionMode::BlockifierOnly;
        self.comparator_enabled = false;
    }

    /// Merge manager-owned policy state with executor-owned runtime replay truth.
    pub fn status(&self, replay_status: RuntimeReplayStatus) -> ExecutionboxStatus {
        ExecutionboxStatus {
            mode: self.mode,
            startup_mode: self.startup_mode,
            startup_recovery_active: self.startup_recovery_active,
            reason: self.reason,
            taint_block: self.taint_block,
            replay_from: replay_status.replay_from,
            replay_to: replay_status.replay_to,
            replay_cursor: replay_status.replay_cursor,
            replay_backlog_empty: replay_status.replay_backlog_empty,
            replay_supported: replay_status.replay_supported,
            comparator_enabled: self.comparator_enabled,
            reexec_epoch: self.reexec_epoch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manager_mixed_startup() -> FallbackManager {
        FallbackManager::new(StartupExecutionMode::Mixed)
    }

    fn make_manager_blockifier_only_startup() -> FallbackManager {
        FallbackManager::new(StartupExecutionMode::BlockifierOnly)
    }

    // --- Startup recovery mapping ---

    #[test]
    fn startup_recovery_active_before_complete() {
        let m = make_manager_mixed_startup();
        assert!(m.startup_recovery_active);
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly, "mode must be BlockifierOnly during recovery");
    }

    #[test]
    fn persistent_fallback_recovery_does_not_restore_mixed_mode() {
        let mut manager = make_manager_mixed_startup();
        manager.on_persistent_fallback_recovery_complete();

        assert!(!manager.startup_recovery_active);
        assert_eq!(manager.mode, ExecutionMode::BlockifierOnly);
        assert!(!manager.comparator_enabled);
    }

    #[test]
    fn startup_mode_mixed_switches_to_mixed_on_recovery_complete() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        assert!(!m.startup_recovery_active);
        assert_eq!(m.mode, ExecutionMode::Mixed);
        assert!(m.comparator_enabled);
    }

    #[test]
    fn startup_mode_blockifier_only_stays_blockifier_only_on_recovery_complete() {
        let mut m = make_manager_blockifier_only_startup();
        m.on_startup_recovery_complete();
        assert!(!m.startup_recovery_active);
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        assert!(!m.comparator_enabled);
    }

    // --- enable/disable semantics ---

    #[test]
    fn enable_while_startup_recovery_active_returns_replay_in_progress() {
        let mut m = make_manager_mixed_startup();
        assert!(m.startup_recovery_active);
        let result = m.executionbox_enable();
        assert_eq!(result, Err(EnableError::ReplayInProgress));
        // Mode must not change
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
    }

    #[test]
    fn enable_when_blockifier_only_and_no_replay_switches_to_mixed() {
        let mut m = make_manager_blockifier_only_startup();
        m.on_startup_recovery_complete();
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        let result = m.executionbox_enable();
        assert_eq!(result, Ok(EnableOutcome::EnabledNow));
        assert_eq!(m.mode, ExecutionMode::Mixed);
        assert!(m.comparator_enabled);
        assert!(m.reason.is_none());
        assert!(m.taint_block.is_none());
    }

    #[test]
    fn enable_when_already_mixed_is_idempotent_success() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        assert_eq!(m.mode, ExecutionMode::Mixed);
        let result = m.executionbox_enable();
        assert_eq!(result, Ok(EnableOutcome::AlreadyMixed));
        assert_eq!(m.mode, ExecutionMode::Mixed);
    }

    #[test]
    fn disable_from_mixed_switches_to_blockifier_only() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        assert_eq!(m.mode, ExecutionMode::Mixed);
        m.executionbox_disable();
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        assert_eq!(m.reason, Some(FallbackReason::ManualForceDisable));
        assert!(!m.comparator_enabled);
    }

    #[test]
    fn disable_is_idempotent_when_already_blockifier_only() {
        let mut m = make_manager_blockifier_only_startup();
        m.on_startup_recovery_complete();
        // Already in BlockifierOnly, disable should be no-op (no reason change)
        let reason_before = m.reason;
        m.executionbox_disable();
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        // Idempotent: reason must not change (no reason was set before)
        assert_eq!(m.reason, reason_before);
    }

    // --- enter_fallback ---

    #[test]
    fn enter_fallback_state_diff_mismatch_sets_mode_and_clears_comparator() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        m.enter_fallback(FallbackReason::StateDiffMismatch, 10);
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        assert_eq!(m.reason, Some(FallbackReason::StateDiffMismatch));
        assert_eq!(m.taint_block, Some(10));
        assert!(!m.comparator_enabled);
    }

    #[test]
    fn enter_fallback_increments_reexec_epoch() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        let epoch_before = m.reexec_epoch;
        m.enter_fallback(FallbackReason::ExecResourcesOverLimit, 5);
        assert_eq!(m.reexec_epoch, epoch_before + 1);
    }

    // --- status snapshot ---

    #[test]
    fn status_has_no_intent_latch_fields() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        let s = m.status(RuntimeReplayStatus::idle());
        // Design: no intent/latch fields in V1
        assert_eq!(s.mode, ExecutionMode::Mixed);
        assert_eq!(s.startup_mode, StartupExecutionMode::Mixed);
        assert!(!s.startup_recovery_active);
        assert!(s.reason.is_none());
        assert!(s.taint_block.is_none());
        assert!(s.replay_from.is_none());
        assert!(s.replay_backlog_empty);
        assert!(s.replay_supported);
        assert!(s.comparator_enabled);
    }

    #[test]
    fn status_uses_executor_owned_runtime_replay_fields() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        m.enter_fallback(FallbackReason::StateDiffMismatch, 42);

        let s = m.status(RuntimeReplayStatus::in_progress());
        assert_eq!(s.mode, ExecutionMode::BlockifierOnly);
        assert_eq!(s.reason, Some(FallbackReason::StateDiffMismatch));
        assert_eq!(s.taint_block, Some(42));
        assert!(!s.replay_backlog_empty);
        assert!(s.replay_supported);
        assert!(!s.comparator_enabled);
    }

    // --- C-006 hardening: comparator pipeline error fallback ---

    #[test]
    fn enter_fallback_comparator_pipeline_error_sets_mode_and_reason() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        assert_eq!(m.mode, ExecutionMode::Mixed);
        assert!(m.comparator_enabled);

        m.enter_fallback(FallbackReason::ComparatorPipelineError, 42);

        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);
        assert_eq!(m.reason, Some(FallbackReason::ComparatorPipelineError));
        assert_eq!(m.taint_block, Some(42));
        assert!(!m.comparator_enabled);
    }

    #[test]
    fn enable_after_comparator_pipeline_error_succeeds_when_no_replay() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        m.enter_fallback(FallbackReason::ComparatorPipelineError, 42);
        assert_eq!(m.mode, ExecutionMode::BlockifierOnly);

        // Control-plane replay gating is now handled outside the manager.
        let result = m.executionbox_enable();
        assert_eq!(result, Ok(EnableOutcome::EnabledNow));
        assert_eq!(m.mode, ExecutionMode::Mixed);
        assert!(m.comparator_enabled);
        assert!(m.reason.is_none());
    }

    #[test]
    fn status_after_comparator_pipeline_error_reflects_blockifier_only() {
        let mut m = make_manager_mixed_startup();
        m.on_startup_recovery_complete();
        m.enter_fallback(FallbackReason::ComparatorPipelineError, 42);

        let s = m.status(RuntimeReplayStatus::idle());
        assert_eq!(s.mode, ExecutionMode::BlockifierOnly);
        assert_eq!(s.reason, Some(FallbackReason::ComparatorPipelineError));
        assert_eq!(s.taint_block, Some(42));
        assert!(!s.comparator_enabled);
        assert!(s.replay_backlog_empty);
        assert!(s.replay_supported);
    }
}
