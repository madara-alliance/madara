use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, strum_macros::Display, Eq)]
pub enum JobStatus {
    /// An acknowledgement that the job has been received by the
    /// orchestrator and is waiting to be processed
    Created,
    /// Some system has taken a lock over the job for processing and no
    /// other system to process the job
    LockedForProcessing,
    /// The job has been processed and is pending verification
    PendingVerification,
    /// The job has been processed and verified. No other actions needs to be taken
    Completed,
    /// The job was processed but the was unable to be verified under the given time
    VerificationTimeout,
    /// The job failed processing
    VerificationFailed,
    /// The job failed completing
    Failed,
    /// The job is being retried
    PendingRetry,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
