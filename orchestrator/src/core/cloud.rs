use super::error::OrchestratorCoreError;
use crate::cli::SetupCmd;
use crate::{cli::RunCmd, types::params::cloud_provider::AWSCredentials};
use aws_config::SdkConfig;
use futures::executor::block_on;
use tracing::{debug, info};

/// Cloud provider
/// This enum represents the different cloud providers that the Orchestrator can interact with.
#[derive(Clone)]
pub enum CloudProvider {
    AWS(Box<SdkConfig>),
}

impl CloudProvider {
    /// get_aws_client_or_panic - Get the AWS client or panic
    ///
    /// # Returns
    ///  the AWS client
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
        f.write_str(self.get_provider_name().as_str())
    }
}

// Implement Display using Debug since they share the same formatting
impl std::fmt::Display for CloudProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.get_provider_name())
    }
}

/// Try from run cmd
///
/// # Arguments
///
/// * `cmd` - The run command
///
/// # Returns
///  the cloud provider based on the run command
///
/// # Errors
/// Returns an error if the provider is not AWS
///
impl TryFrom<RunCmd> for CloudProvider {
    type Error = OrchestratorCoreError;

    fn try_from(cmd: RunCmd) -> Result<Self, Self::Error> {
        if cmd.aws_config_args.aws {
            debug!("Initializing AWS configuration");
            let aws_cred = AWSCredentials::from(cmd.aws_config_args.clone());
            let config = block_on(aws_cred.get_aws_config());
            info!("Successfully created AWS configuration");
            Ok(CloudProvider::AWS(Box::new(config)))
        } else {
            Err(OrchestratorCoreError::InvalidProvider("AWS provider is not enabled".to_string()))
        }
    }
}

/// Try from Setup cmd
///
/// # Arguments
///
/// * `cmd` - The run command
///
/// # Returns
/// Returns the cloud provider based on the run command
///
/// # Errors
/// Returns an error if the provider is not AWS
///
impl TryFrom<SetupCmd> for CloudProvider {
    type Error = OrchestratorCoreError;

    fn try_from(cmd: SetupCmd) -> Result<Self, Self::Error> {
        if cmd.aws_config_args.aws {
            debug!("Initializing AWS configuration from setup command");
            let aws_cred = AWSCredentials::from(cmd.aws_config_args.clone());
            let config = block_on(aws_cred.get_aws_config());
            info!("Successfully created AWS configuration");
            Ok(CloudProvider::AWS(Box::new(config)))
        } else {
            Err(OrchestratorCoreError::InvalidProvider("AWS provider is not enabled".to_string()))
        }
    }
}

/// Try from AWSCredentials
impl TryFrom<AWSCredentials> for CloudProvider {
    type Error = OrchestratorCoreError;

    fn try_from(aws_cred: AWSCredentials) -> Result<Self, Self::Error> {
        debug!("Initializing AWS configuration from credentials");
        let config = block_on(aws_cred.get_aws_config());
        info!("Successfully created AWS configuration");
        Ok(CloudProvider::AWS(Box::new(config)))
    }
}
