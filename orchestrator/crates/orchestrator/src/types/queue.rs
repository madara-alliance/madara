use crate::types::jobs::types::JobType;
use strum_macros::{Display, EnumIter};

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
    #[strum(serialize = "job_handle_failure")]
    JobHandleFailure,
    #[strum(serialize = "worker_trigger")]
    WorkerTrigger,
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
        }
    }
    fn verify_queue_name(&self) -> QueueType {
        match self {
            JobType::SnosRun => QueueType::SnosJobVerification,
            JobType::ProofCreation => QueueType::ProvingJobVerification,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobVerification,
            JobType::DataSubmission => QueueType::DataSubmissionJobVerification,
            JobType::StateTransition => QueueType::UpdateStateJobVerification,
        }
    }
}
