use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    #[serde(default)]
    pub min_block_to_process: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_block_to_process: Option<u64>,

    #[serde(default = "default_max_created_snos")]
    pub max_concurrent_created_snos_jobs: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_snos_jobs: Option<usize>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_proving_jobs: Option<usize>,

    #[serde(default = "default_job_timeout")]
    pub job_processing_timeout_seconds: u64,
}

fn default_max_created_snos() -> u64 {
    200
}
fn default_job_timeout() -> u64 {
    1800
}
