use crate::{cli::RunCmd, error::OrchestratorResult, types::params::cloud_provider::AWSCredentials};
use aws_config::SdkConfig;
use futures::executor::block_on;

use super::error::OrchestratorCoreError;

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

impl TryFrom<RunCmd> for CloudProvider {
    type Error = OrchestratorCoreError;

    fn try_from(cmd: RunCmd) -> Result<Self, Self::Error> {
        if cmd.aws_config_args.aws {
            let aws_cred = AWSCredentials::from(cmd.aws_config_args.clone());
            let config = block_on(aws_cred.get_aws_config());
            Ok(CloudProvider::AWS(Box::new(config)))
        } else {
            Err(OrchestratorCoreError::InvalidProvider("AWS".to_string()))
        }

    }
}
