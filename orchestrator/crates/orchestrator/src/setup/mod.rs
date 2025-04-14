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

pub mod old {
    use std::time::Duration;

    use async_std::task::sleep;
    use aws_config::SdkConfig;

    use crate::alerts::aws_sns::AWSSNS;
    use crate::alerts::Alerts;
    use crate::cli::alert::AlertValidatedArgs;
    use crate::cli::cron::CronValidatedArgs;
    use crate::cli::queue::QueueValidatedArgs;
    use crate::cli::storage::StorageValidatedArgs;
    use crate::cli::SetupCmd;
    use crate::config::build_provider_config;
    use crate::cron::event_bridge::AWSEventBridge;
    use crate::cron::Cron;
    use crate::data_storage::aws_s3::AWSS3;
    use crate::data_storage::DataStorage;
    use crate::queue::sqs::SqsQueue;
    use crate::queue::QueueProvider as _;

    #[derive(Clone)]
    pub enum SetupConfig {
        AWS(SdkConfig),
    }

    // Note: we are using println! instead of tracing::info! because telemetry is not yet initialized
    // and it get initialized during the run_orchestrator function.
    pub async fn setup_cloud(setup_cmd: &SetupCmd) -> color_eyre::Result<()> {
        // AWS
        println!("Setting up cloud. ⏳");
        let provider_params = setup_cmd.validate_provider_params().expect("Failed to validate provider params");
        let provider_config = build_provider_config(&provider_params).await;
        let aws_config = provider_config.get_aws_client_or_panic();

        // Queues
        println!("Setting up queues. ⏳");
        let queue_params = setup_cmd.validate_queue_params().expect("Failed to validate queue params");
        match queue_params {
            QueueValidatedArgs::AWSSQS(aws_sqs_params) => {
                let sqs = Box::new(SqsQueue::new_with_args(aws_sqs_params, aws_config));
                sqs.setup().await?
            }
        }
        println!("Queues setup completed ✅");

        // Waiting for few seconds to let AWS index the queues
        sleep(Duration::from_secs(20)).await;

        // Data Storage
        println!("Setting up data storage. ⏳");
        let data_storage_params = setup_cmd.validate_storage_params().expect("Failed to validate storage params");

        match data_storage_params {
            StorageValidatedArgs::AWSS3(aws_s3_params) => {
                let s3 = Box::new(AWSS3::new_with_args(&aws_s3_params, aws_config).await);
                s3.setup(&StorageValidatedArgs::AWSS3(aws_s3_params.clone())).await?
            }
        }
        println!("Data storage setup completed ✅");

        // Cron
        println!("Setting up cron. ⏳");
        // Sleeping for few seconds to let AWS index the newly created queues to be used for setting up cron
        sleep(Duration::from_secs(60)).await;
        let cron_params = setup_cmd.validate_cron_params().expect("Failed to validate cron params");
        match cron_params {
            CronValidatedArgs::AWSEventBridge(aws_event_bridge_params) => {
                let aws_config = provider_config.get_aws_client_or_panic();
                let event_bridge = Box::new(AWSEventBridge::new_with_args(&aws_event_bridge_params, aws_config));
                event_bridge.setup().await?
            }
        }
        println!("Cron setup completed ✅");

        // Alerts
        println!("Setting up alerts. ⏳");
        let alert_params = setup_cmd.validate_alert_params().expect("Failed to validate alert params");
        match alert_params {
            AlertValidatedArgs::AWSSNS(aws_sns_params) => {
                let aws_config = provider_config.get_aws_client_or_panic();
                let sns = Box::new(AWSSNS::new_with_args(&aws_sns_params, aws_config).await);
                sns.setup().await?
            }
        }
        println!("Alerts setup completed ✅");

        Ok(())
    }
}
