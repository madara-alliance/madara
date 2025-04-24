use crate::cli::provider::{AWSConfigValidatedArgs, ProviderValidatedArgs};
use crate::cli::SetupCmd;
use crate::core::cloud::CloudProvider;
use crate::setup::factory::ResourceFactory;
use crate::types::params::{AlertArgs, CronArgs, QueueArgs, StorageArgs};
use crate::{OrchestratorError, OrchestratorResult};
use aws_config::Region;
use aws_credential_types::Credentials;
use std::sync::Arc;
use tracing::{info, instrument};

pub(crate) mod aws;
mod creator;
pub(crate) mod factory;
pub(crate) mod queue;
mod wrapper;

/// Setup function that initializes all necessary resources
#[instrument]
pub async fn setup(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    let cloud_provider = setup_cloud_provider(&setup_cmd).await?;

    info!("Setting up resources for Orchestrator...");

    let queue_params = QueueArgs::try_from(setup_cmd.clone())?;
    let storage_params = StorageArgs::try_from(setup_cmd.clone())?;
    let alert_params = AlertArgs::try_from(setup_cmd.clone())?;
    let cron_params = CronArgs::try_from(setup_cmd.clone())?;

    let resources = match cloud_provider.clone().get_provider_name().as_str() {
        "AWS" => ResourceFactory::new_with_aws(cloud_provider, queue_params, cron_params, storage_params, alert_params),
        a => Err(OrchestratorError::InvalidCloudProviderError(a.to_string()))?,
    };
    resources.setup_resource().await?;

    Ok(())
}

// REVIEW: 18 : I think we should match against setup_cmd.aws it should be bool true, make it generalised

/// Set up the orchestrator with the provided configuration
pub async fn setup_cloud_provider(setup_cmd: &SetupCmd) -> OrchestratorResult<Arc<CloudProvider>> {
    let aws_config = AWSConfigValidatedArgs::try_from(setup_cmd.clone())?;

    // Create AWS SDK config
    let sdk_config = aws_config::from_env()
        .region(Region::new(aws_config.aws_region.clone()))
        .credentials_provider(Credentials::from_keys(
            &aws_config.aws_access_key_id,
            &aws_config.aws_secret_access_key,
            None,
        ))
        .load()
        .await;
    info!("AWS credentials validated successfully");

    let cloud_provider = Arc::new(CloudProvider::AWS(Box::new(sdk_config)));

    Ok(cloud_provider)
}
