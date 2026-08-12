use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Mixed,
    BlockifierOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupExecutionMode {
    Mixed,
    BlockifierOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackReason {
    OutputMismatch,
    StateDiffMismatch,
    ExecResourcesOverLimit,
    ManualForceDisable,
    ComparatorPipelineError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReplayStatus {
    pub execution_epoch: u64,
    pub replay_from: Option<u64>,
    pub replay_to: Option<u64>,
    pub replay_cursor: Option<u64>,
    pub replay_backlog_empty: bool,
    pub replay_supported: bool,
}

impl RuntimeReplayStatus {
    pub const fn idle() -> Self {
        Self::idle_for_epoch(0)
    }

    pub const fn idle_for_epoch(execution_epoch: u64) -> Self {
        Self {
            execution_epoch,
            replay_from: None,
            replay_to: None,
            replay_cursor: None,
            replay_backlog_empty: true,
            replay_supported: true,
        }
    }

    pub const fn in_progress() -> Self {
        Self::in_progress_for_epoch(0)
    }

    pub const fn in_progress_for_epoch(execution_epoch: u64) -> Self {
        Self {
            execution_epoch,
            replay_from: None,
            replay_to: None,
            replay_cursor: None,
            replay_backlog_empty: false,
            replay_supported: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionboxStatus {
    pub mode: ExecutionMode,
    pub startup_mode: StartupExecutionMode,
    pub startup_recovery_active: bool,
    pub reason: Option<FallbackReason>,
    pub taint_block: Option<u64>,
    pub replay_from: Option<u64>,
    pub replay_to: Option<u64>,
    pub replay_cursor: Option<u64>,
    pub replay_backlog_empty: bool,
    pub replay_supported: bool,
    pub comparator_enabled: bool,
    pub reexec_epoch: u64,
}
