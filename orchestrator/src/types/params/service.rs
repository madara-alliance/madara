use crate::cli::server::ServerCliArgs;
use crate::cli::service::ServiceCliArgs;

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub max_block_to_process: Option<u64>,
    pub min_block_to_process: u64,
    pub max_concurrent_created_snos_jobs: u64,
    pub max_concurrent_created_proving_jobs: u64,
    pub max_concurrent_created_aggregator_jobs: u64,

    /// Per-orchestrator concurrency limits for processing phase
    pub max_concurrent_snos_jobs_processing: Option<usize>,
    pub max_concurrent_proving_jobs_processing: Option<usize>,
    pub max_concurrent_aggregator_jobs_processing: Option<usize>,
    pub max_concurrent_data_submission_jobs_processing: Option<usize>,

    /// Per-orchestrator concurrency limits for verification phase
    pub max_concurrent_snos_jobs_verification: Option<usize>,
    pub max_concurrent_proving_jobs_verification: Option<usize>,
    pub max_concurrent_aggregator_jobs_verification: Option<usize>,
    pub max_concurrent_data_submission_jobs_verification: Option<usize>,

    pub job_processing_timeout_seconds: u64,
    /// Polling interval when no jobs available (milliseconds)
    pub poll_interval_ms: u64,
}

impl From<ServiceCliArgs> for ServiceParams {
    fn from(args: ServiceCliArgs) -> Self {
        Self {
            max_block_to_process: args.max_block_to_process,
            min_block_to_process: args.min_block_to_process,
            max_concurrent_created_snos_jobs: args.max_concurrent_created_snos_jobs,
            max_concurrent_created_proving_jobs: args.max_concurrent_created_proving_jobs,
            max_concurrent_created_aggregator_jobs: args.max_concurrent_created_aggregator_jobs,

            max_concurrent_snos_jobs_processing: args.max_concurrent_snos_jobs_processing,
            max_concurrent_snos_jobs_verification: args.max_concurrent_snos_jobs_verification,
            max_concurrent_proving_jobs_processing: args.max_concurrent_proving_jobs_processing,
            max_concurrent_proving_jobs_verification: args.max_concurrent_proving_jobs_verification,
            max_concurrent_aggregator_jobs_processing: args.max_concurrent_aggregator_jobs_processing,
            max_concurrent_aggregator_jobs_verification: args.max_concurrent_aggregator_jobs_verification,
            max_concurrent_data_submission_jobs_processing: args.max_concurrent_data_submission_jobs_processing,
            max_concurrent_data_submission_jobs_verification: args.max_concurrent_data_submission_jobs_verification,

            job_processing_timeout_seconds: args.job_processing_timeout_seconds,
            poll_interval_ms: args.poll_interval_ms,
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
