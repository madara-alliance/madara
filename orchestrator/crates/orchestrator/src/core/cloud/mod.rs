use crate::error::OrchestratorResult;
use aws_config::SdkConfig;

/// Cloud provider
/// This enum represents the different cloud providers that the Orchestrator can interact with.
#[derive(Debug, Clone)]
pub enum CloudProvider {
    AWS(Box<SdkConfig>),
}

impl CloudProvider {
    /// Get the AWS SDK config
    ///
    /// # Returns :
    /// - `Ok(SdkConfig)` if the provider is AWS
    /// - `Err(Error)` if the provider is not AWS
    ///
    /// Returns the AWS SDK config if the provider is AWS.
    pub fn get_aws_config(&self) -> OrchestratorResult<&SdkConfig> {
        match self {
            Self::AWS(config) => Ok(config),
        }
    }
}
