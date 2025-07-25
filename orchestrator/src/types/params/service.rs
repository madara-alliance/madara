use crate::cli::server::ServerCliArgs;
use crate::cli::service::ServiceCliArgs;

#[derive(Debug, Clone)]
pub struct ServiceParams {
    pub max_block_to_process: Option<u64>,
    pub min_block_to_process: u64,
    pub max_concurrent_created_snos_jobs: u64,
    pub max_concurrent_snos_jobs: Option<usize>,
    pub max_concurrent_proving_jobs: Option<usize>,
    pub job_processing_timeout_seconds: u64,
}

impl From<ServiceCliArgs> for ServiceParams {
    fn from(args: ServiceCliArgs) -> Self {
        Self {
            max_block_to_process: args.max_block_to_process,
            min_block_to_process: args.min_block_to_process,
            max_concurrent_created_snos_jobs: args.max_concurrent_created_snos_jobs,
            max_concurrent_snos_jobs: args.max_concurrent_snos_jobs,
            max_concurrent_proving_jobs: args.max_concurrent_proving_jobs,
            job_processing_timeout_seconds: args.job_processing_timeout_seconds,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ServerParams {
    pub host: String,
    pub port: u16,
}

impl From<ServerCliArgs> for ServerParams {
    fn from(value: ServerCliArgs) -> Self {
        Self { host: value.host, port: value.port }
    }
}
