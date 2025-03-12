use crate::error::{OrchestratorError, OrchestratorResult};
use std::env;

// Environment variable names
pub const ENV_PREFIX: &str = "MADARA_ORCHESTRATOR_PREFIX";
pub const ENV_S3_BUCKET_NAME: &str = "MADARA_ORCHESTRATOR_AWS_S3_BUCKET_NAME";
pub const ENV_SQS_BASE_QUEUE_URL: &str = "MADARA_ORCHESTRATOR_SQS_BASE_QUEUE_URL";
pub const ENV_SQS_SUFFIX: &str = "MADARA_ORCHESTRATOR_SQS_SUFFIX";
pub const ENV_AWS_REGION: &str = "AWS_REGION";

/// Get the orchestrator prefix from environment variables
pub fn get_prefix() -> OrchestratorResult<String> {
    env::var(ENV_PREFIX)
        .map_err(|_| OrchestratorError::ConfigError(format!("{} environment variable is not set", ENV_PREFIX)))
}

/// Get the S3 bucket name with prefix from environment variables
pub fn get_s3_bucket_name() -> OrchestratorResult<String> {
    let prefix = get_prefix()?;
    let bucket_name = env::var(ENV_S3_BUCKET_NAME).unwrap_or_else(|_| "bucket".to_string());

    Ok(format!("{}-{}", prefix, bucket_name))
}

/// Get the AWS region from environment variables
pub fn get_aws_region() -> OrchestratorResult<String> {
    env::var(ENV_AWS_REGION)
        .map_err(|_| OrchestratorError::ConfigError(format!("{} environment variable is not set", ENV_AWS_REGION)))
}

/// Get the SQS queue name with prefix
pub fn get_sqs_queue_name(base_name: &str) -> OrchestratorResult<String> {
    let prefix = get_prefix()?;
    let suffix = env::var(ENV_SQS_SUFFIX).unwrap_or_else(|_| "queue".to_string());

    Ok(format!("{}-{}-{}", prefix, base_name, suffix))
}

/// Get the SQS base URL from environment variables
pub fn get_sqs_base_url() -> OrchestratorResult<String> {
    env::var(ENV_SQS_BASE_QUEUE_URL).map_err(|_| {
        OrchestratorError::ConfigError(format!("{} environment variable is not set", ENV_SQS_BASE_QUEUE_URL))
    })
}
