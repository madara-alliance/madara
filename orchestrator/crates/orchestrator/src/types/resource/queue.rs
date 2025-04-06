use strum_macros::{Display, EnumIter};


/// TODO: we have duplicate for this enum use that instead of this
#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter)]
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