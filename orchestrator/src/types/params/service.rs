use crate::cli::server::ServerCliArgs;
use crate::cli::service::ServiceCliArgs;
use crate::types::jobs::types::JobType;

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub max_block_to_process: Option<u64>,
    pub min_block_to_process: u64,
    pub max_concurrent_created_snos_jobs: u64,
    pub max_concurrent_snos_jobs: Option<usize>,
    pub max_concurrent_proving_jobs: Option<usize>,
    pub snos_job_timeout_seconds: u64,
    pub proving_job_timeout_seconds: u64,
    pub proof_registration_timeout_seconds: u64,
    pub data_submission_timeout_seconds: u64,
    pub state_transition_timeout_seconds: u64,
    pub aggregator_job_timeout_seconds: u64,
    pub snos_job_buffer_size: u64,
    pub max_priority_queue_size: usize,
}

impl ServiceParams {
    /// Get the timeout for a specific job type
    ///
    /// # Arguments
    /// * `job_type` - The type of job to get the timeout for
    ///
    /// # Returns
    /// The timeout in seconds for the specified job type
    pub fn get_job_timeout(&self, job_type: &JobType) -> u64 {
        match job_type {
            JobType::SnosRun => self.snos_job_timeout_seconds,
            JobType::ProofCreation => self.proving_job_timeout_seconds,
            JobType::ProofRegistration => self.proof_registration_timeout_seconds,
            JobType::DataSubmission => self.data_submission_timeout_seconds,
            JobType::StateTransition => self.state_transition_timeout_seconds,
            JobType::Aggregator => self.aggregator_job_timeout_seconds,
        }
    }
}

impl From<ServiceCliArgs> for ServiceParams {
    fn from(args: ServiceCliArgs) -> Self {
        Self {
            max_block_to_process: args.max_block_to_process,
            min_block_to_process: args.min_block_to_process,
            max_concurrent_created_snos_jobs: args.max_concurrent_created_snos_jobs,
            max_concurrent_snos_jobs: args.max_concurrent_snos_jobs,
            max_concurrent_proving_jobs: args.max_concurrent_proving_jobs,
            snos_job_timeout_seconds: args.snos_job_timeout_seconds,
            proving_job_timeout_seconds: args.proving_job_timeout_seconds,
            proof_registration_timeout_seconds: args.proof_registration_timeout_seconds,
            data_submission_timeout_seconds: args.data_submission_timeout_seconds,
            state_transition_timeout_seconds: args.state_transition_timeout_seconds,
            aggregator_job_timeout_seconds: args.aggregator_job_timeout_seconds,
            snos_job_buffer_size: args.snos_job_buffer_size,
            max_priority_queue_size: args.max_priority_queue_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerParams {
    pub host: String,
    pub port: u16,
    pub admin_enabled: bool,
}

impl From<ServerCliArgs> for ServerParams {
    fn from(value: ServerCliArgs) -> Self {
        Self { host: value.host, port: value.port, admin_enabled: value.admin_enabled }
    }
}
