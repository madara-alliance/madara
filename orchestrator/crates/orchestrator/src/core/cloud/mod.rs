use crate::error::OrchestratorResult;
use aws_config::SdkConfig;

/// Cloud provider
/// This enum represents the different cloud providers that the Orchestrator can interact with.
#[derive(Clone)]
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
    pub fn get_aws_client_or_panic(&self) -> &SdkConfig {
        match self {
            CloudProvider::AWS(config) => config.as_ref(),
        }
    }

    pub fn get_provider_name(&self) -> String {
        match self {
            CloudProvider::AWS(_) => "AWS".to_string(),
        }
    }
}

impl std::fmt::Debug for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudProvider::AWS(_) => write!(f, "AWS"),
        }
    }
}

impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloudProvider::AWS(_) => write!(f, "AWS"),
        }
    }
}
