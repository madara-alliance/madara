use crate::cli::SetupCmd;
use crate::core::client::lock::mongodb::MongoLockClient;
use crate::core::client::MongoDbClient;
use crate::core::cloud::CloudProvider;
use crate::setup::factory::ResourceFactory;
use crate::types::params::database::DatabaseArgs;
use crate::types::params::{AlertArgs, CronArgs, MiscellaneousArgs, QueueArgs, StorageArgs};
use crate::{OrchestratorError, OrchestratorResult};
use std::sync::Arc;
use tracing::debug;
use tracing::info;

pub(crate) mod aws;
mod creator;
pub(crate) mod factory;
mod wrapper;

/// Setup function that initializes all necessary resources
pub async fn setup(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    let cloud_provider = setup_cloud_provider(setup_cmd).await?;

    info!("Setting up resources for Orchestrator...");

    // Setup database indexes (only if MongoDB args are provided)
    if setup_cmd.mongodb_args.mongodb_connection_url.is_some() {
        setup_database(setup_cmd).await?;
    } else {
        info!("Skipping MongoDB setup (--mongodb not provided)");
    }

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

/// Setup database indexes for all collections
async fn setup_database(setup_cmd: &SetupCmd) -> OrchestratorResult<()> {
    info!("Setting up MongoDB indexes...");

    let db_args = DatabaseArgs::try_from(setup_cmd.clone())?;

    // Create indexes for jobs, aggregator_batches, and snos_batches collections
    let db_client = MongoDbClient::new(&db_args).await?;
    db_client.ensure_indexes().await?;
    info!("Created indexes for jobs, aggregator_batches, and snos_batches collections");

    // Create indexes for locks collection
    let lock_client = MongoLockClient::new(&db_args).await?;
    lock_client.ensure_indexes().await?;
    info!("Created indexes for locks collection");

    info!("MongoDB indexes setup complete");
    Ok(())
}

/// Set up the orchestrator with the provided configuration
pub async fn setup_cloud_provider(setup_cmd: &SetupCmd) -> OrchestratorResult<Arc<CloudProvider>> {
    let cloud_provider = CloudProvider::try_from(setup_cmd.clone())
        .map_err(|e| OrchestratorError::InvalidCloudProviderError(e.to_string()))?;

    info!("Cloud Provider initialized - AWS");

    Ok(Arc::new(cloud_provider))
}
