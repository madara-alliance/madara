use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Default)]
pub enum JobStatus {
    // ========================================
    // WAITING STATES (claimed_by = null)
    // ========================================
    /// New job, waiting to be picked up for processing
    #[default]
    Created,

    /// Processing complete, waiting to be picked up for verification
    Processed,

    /// Processing failed, auto-retry pending (will be picked up again)
    PendingRetryProcessing,

    /// Verification failed, auto-retry pending (will be picked up again)
    PendingRetryVerification,

    // ========================================
    // WORKING STATES (claimed_by = orchestrator_id)
    // ========================================
    /// Job is being processed by an orchestrator
    LockedForProcessing,

    /// Job is being verified by an orchestrator
    LockedForVerification,

    // ========================================
    // TERMINAL STATES (require manual intervention)
    // ========================================
    /// Verification complete, job finished successfully
    Completed,

    /// Processing failed after max retries (manual intervention required)
    ProcessingFailed,

    /// Verification failed after max retries (manual intervention required)
    VerificationFailed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobStatus::Created => write!(f, "Created"),
            JobStatus::Processed => write!(f, "Processed"),
            JobStatus::PendingRetryProcessing => write!(f, "PendingRetryProcessing"),
            JobStatus::PendingRetryVerification => write!(f, "PendingRetryVerification"),
            JobStatus::LockedForProcessing => write!(f, "LockedForProcessing"),
            JobStatus::LockedForVerification => write!(f, "LockedForVerification"),
            JobStatus::Completed => write!(f, "Completed"),
            JobStatus::ProcessingFailed => write!(f, "ProcessingFailed"),
            JobStatus::VerificationFailed => write!(f, "VerificationFailed"),
        }
    }
}

impl JobStatus {
    /// Returns true if the job is in a working state (being processed/verified)
    pub fn is_locked(&self) -> bool {
        matches!(self, JobStatus::LockedForProcessing | JobStatus::LockedForVerification)
    }

    /// Returns true if the job is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobStatus::Completed | JobStatus::ProcessingFailed | JobStatus::VerificationFailed)
    }

    /// Returns true if the job is waiting to be picked up
    pub fn is_waiting(&self) -> bool {
        matches!(
            self,
            JobStatus::Created
                | JobStatus::Processed
                | JobStatus::PendingRetryProcessing
                | JobStatus::PendingRetryVerification
        )
    }

    /// Returns true if the job can be picked up for processing
    pub fn can_process(&self) -> bool {
        matches!(self, JobStatus::Created | JobStatus::PendingRetryProcessing)
    }

    /// Returns true if the job can be picked up for verification
    pub fn can_verify(&self) -> bool {
        matches!(self, JobStatus::Processed | JobStatus::PendingRetryVerification)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum JobType {
    /// Running SNOS for a block
    SnosRun,
    /// Submitting DA data to the DA layer
    DataSubmission,
    /// Getting a proof from the proving service
    ProofCreation,
    /// Verifying the proof on the base layer
    ProofRegistration,
    /// Updating the state root on the base layer
    StateTransition,
    /// Aggregating the batches
    Aggregator,
}
