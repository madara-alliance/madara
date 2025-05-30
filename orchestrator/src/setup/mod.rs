use crate::cli::SetupCmd;
use crate::core::cloud::CloudProvider;
use crate::setup::factory::ResourceFactory;
use crate::types::params::{AlertArgs, CronArgs, MiscellaneousArgs, QueueArgs, StorageArgs};
use crate::{OrchestratorError, OrchestratorResult};
use std::sync::Arc;
use tracing::debug;
use tracing::info;

pub(crate) mod aws;
mod creator;
pub(crate) mod factory;
pub(crate) mod queue;
mod wrapper;

/// Setup function that initializes all necessary resources
pub async fn setup(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    let cloud_provider = setup_cloud_provider(setup_cmd).await?;

    info!("Setting up resources for Orchestrator...");

    let queue_params = QueueArgs::try_from(setup_cmd.clone())?;
    let storage_params = StorageArgs::try_from(setup_cmd.clone())?;
    let alert_params = AlertArgs::try_from(setup_cmd.clone())?;
    let cron_params = CronArgs::try_from(setup_cmd.clone())?;
    let miscellaneous_params = MiscellaneousArgs::try_from(setup_cmd.clone())?;

    debug!("Queue Params: {:?}", queue_params);
    debug!("Storage Params: {:?}", storage_params);
    debug!("Alert Params: {:?}", alert_params);
    debug!("Cron Params: {:?}", cron_params);

    let resources = match cloud_provider.clone().get_provider_name().as_str() {
        "AWS" => ResourceFactory::new_with_aws(
            cloud_provider,
            queue_params,
            cron_params,
            storage_params,
            alert_params,
            miscellaneous_params,
        ),
        cloud_provider => Err(OrchestratorError::InvalidCloudProviderError(cloud_provider.to_string()))?,
    };
    resources.setup_resource(&setup_cmd.layer).await?;

    Ok(())
}

/// Set up the orchestrator with the provided configuration
pub async fn setup_cloud_provider(setup_cmd: &SetupCmd) -> OrchestratorResult<Arc<CloudProvider>> {
    let cloud_provider = CloudProvider::try_from(setup_cmd.clone())
        .map_err(|e| OrchestratorError::InvalidCloudProviderError(e.to_string()))?;

    info!("Cloud Provider initialized - AWS");

    Ok(Arc::new(cloud_provider))
}
