use crate::error::event::EventSystemError;
use crate::types::jobs::types::JobType;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumIter};

#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter, Hash)]
pub enum JobState {
    Processing,
    Verification,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum JobAction {
    Process,
    Verify,
}

#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter, Hash)]
pub enum QueueType {
    #[strum(serialize = "snos_job_processing")]
    SnosJobProcessing,
    #[strum(serialize = "snos_job_verification")]
    SnosJobVerification,
    #[strum(serialize = "proving_job_processing")]
    ProvingJobProcessing,
    #[strum(serialize = "proving_job_verification")]
    ProvingJobVerification,
    #[strum(serialize = "proof_registration_job_processing")]
    ProofRegistrationJobProcessing,
    #[strum(serialize = "proof_registration_job_verification")]
    ProofRegistrationJobVerification,
    #[strum(serialize = "data_submission_job_processing")]
    DataSubmissionJobProcessing,
    #[strum(serialize = "data_submission_job_verification")]
    DataSubmissionJobVerification,
    #[strum(serialize = "update_state_job_processing")]
    UpdateStateJobProcessing,
    #[strum(serialize = "update_state_job_verification")]
    UpdateStateJobVerification,
    #[strum(serialize = "aggregator_job_processing")]
    AggregatorJobProcessing,
    #[strum(serialize = "aggregator_job_verification")]
    AggregatorJobVerification,
    #[strum(serialize = "job_handle_failure")]
    JobHandleFailure,
    #[strum(serialize = "worker_trigger")]
    WorkerTrigger,
    #[strum(serialize = "priority_job_queue")]
    PriorityJobQueue,
}

impl TryFrom<QueueType> for JobState {
    type Error = EventSystemError;
    fn try_from(value: QueueType) -> Result<Self, Self::Error> {
        let state = match value {
            QueueType::SnosJobProcessing => JobState::Processing,
            QueueType::SnosJobVerification => JobState::Verification,
            QueueType::ProvingJobProcessing => JobState::Processing,
            QueueType::ProvingJobVerification => JobState::Verification,
            QueueType::ProofRegistrationJobProcessing => JobState::Processing,
            QueueType::ProofRegistrationJobVerification => JobState::Verification,
            QueueType::DataSubmissionJobProcessing => JobState::Processing,
            QueueType::DataSubmissionJobVerification => JobState::Verification,
            QueueType::UpdateStateJobProcessing => JobState::Processing,
            QueueType::UpdateStateJobVerification => JobState::Verification,
            QueueType::AggregatorJobProcessing => JobState::Processing,
            QueueType::AggregatorJobVerification => JobState::Verification,
            QueueType::JobHandleFailure => Err(Self::Error::InvalidJobType(QueueType::JobHandleFailure.to_string()))?,
            QueueType::WorkerTrigger => Err(Self::Error::InvalidJobType(QueueType::WorkerTrigger.to_string()))?,
            QueueType::PriorityJobQueue => Err(Self::Error::InvalidJobType(QueueType::PriorityJobQueue.to_string()))?,
        };
        Ok(state)
    }
}

pub trait QueueNameForJobType {
    fn process_queue_name(&self) -> QueueType;
    fn verify_queue_name(&self) -> QueueType;
}

impl QueueNameForJobType for JobType {
    fn process_queue_name(&self) -> QueueType {
        match self {
            JobType::SnosRun => QueueType::SnosJobProcessing,
            JobType::ProofCreation => QueueType::ProvingJobProcessing,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobProcessing,
            JobType::DataSubmission => QueueType::DataSubmissionJobProcessing,
            JobType::StateTransition => QueueType::UpdateStateJobProcessing,
            JobType::Aggregator => QueueType::AggregatorJobProcessing,
        }
    }
    fn verify_queue_name(&self) -> QueueType {
        match self {
            JobType::SnosRun => QueueType::SnosJobVerification,
            JobType::ProofCreation => QueueType::ProvingJobVerification,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobVerification,
            JobType::DataSubmission => QueueType::DataSubmissionJobVerification,
            JobType::StateTransition => QueueType::UpdateStateJobVerification,
            JobType::Aggregator => QueueType::AggregatorJobVerification,
        }
    }
}
