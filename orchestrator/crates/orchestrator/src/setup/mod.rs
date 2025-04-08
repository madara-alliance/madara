use std::process::Command;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use aws_config::SdkConfig;
use strum_macros::{Display, EnumIter};
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
use async_trait::async_trait;

#[derive(Clone)]
pub enum SetupConfig {
    AWS(SdkConfig),
}

#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter, Hash)]
pub enum ResourceType {
    #[strum(serialize = "queues")]
    Queues,
    #[strum(serialize = "data_storage")]
    DataStorage,
    #[strum(serialize = "cron")]
    Cron,
    #[strum(serialize = "alerts")]
    Alerts,
}

#[async_trait]
pub trait ResourceStatus {
    async fn are_all_ready(&self) -> bool;
}

lazy_static::lazy_static! {
    static ref RESOURCE_STATUS: Mutex<HashMap<ResourceType, bool>> = Mutex::new(HashMap::new());
}

pub fn update_resource_status(resource_type: ResourceType, value: bool) {
    let mut resource_status = RESOURCE_STATUS.lock().unwrap();
    (*resource_status).insert(resource_type, value);
}

pub fn get_resource_status(resource_type: ResourceType) -> bool {
    let resource_status = RESOURCE_STATUS.lock().unwrap();
    resource_status.get(&resource_type).copied().unwrap_or(false)
}

async fn poll(resource: &dyn ResourceStatus, resource_type: ResourceType, poll_interval: u64, timeout: u64) -> bool {
    let start_time = std::time::Instant::now();
    let timeout_duration = Duration::from_secs(timeout);
    let poll_duration = Duration::from_secs(poll_interval);

    while start_time.elapsed() < timeout_duration {
        println!("polling for {}", resource_type);
        if resource.are_all_ready().await {
            update_resource_status(resource_type, true);
            return true;
        } else {
            tokio::time::sleep(poll_duration).await;
        }
    }
    false
}

// Note: we are using println! instead of tracing::info! because telemetry is not yet initialized
// and it get initialized during the run_orchestrator function.
pub async fn setup_cloud(setup_cmd: &SetupCmd) -> color_eyre::Result<()> {
    update_resource_status(ResourceType::Queues, false);
    update_resource_status(ResourceType::DataStorage, false);
    update_resource_status(ResourceType::Cron, false);
    update_resource_status(ResourceType::Alerts, false);

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
            sqs.setup().await?;
            if poll(&*sqs, ResourceType::Queues, setup_cmd.poll_interval.unwrap(), setup_cmd.timeout.unwrap()).await {
                println!("Queues setup completed ✅");
            } else {
                println!("Queues setup failed ❌");
            }
        }
    }

    // Data Storage
    println!("Setting up data storage. ⏳");
    let data_storage_params = setup_cmd.validate_storage_params().expect("Failed to validate storage params");

    match data_storage_params {
        StorageValidatedArgs::AWSS3(aws_s3_params) => {
            let s3 = Box::new(AWSS3::new_with_args(&aws_s3_params, aws_config).await);
            s3.setup(&StorageValidatedArgs::AWSS3(aws_s3_params.clone())).await?;
            if poll(&*s3, ResourceType::DataStorage, setup_cmd.poll_interval.unwrap(), setup_cmd.timeout.unwrap()).await {
                println!("Data storage setup completed ✅");
            } else {
                println!("Data storage setup failed ❌");
            }
        }
    }

    // Cron
    // Dependencies - Queues
    // We have to make sure that the cron is created only after the queues have been created
    println!("Setting up cron. ⏳");
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
            sns.setup().await?;
            if poll(&*sns, ResourceType::Alerts, setup_cmd.poll_interval.unwrap(), setup_cmd.timeout.unwrap()).await {
                println!("Alerts setup completed ✅");
            } else {
                println!("Alerts setup failed ❌");
            }
        }
    }
    println!("Alerts setup completed ✅");

    Ok(())
}

pub async fn setup_db() -> color_eyre::Result<()> {
    // We run the js script in the folder root:
    println!("Setting up database.");

    Command::new("node").arg("migrate-mongo-config.js").output()?;

    println!("Database setup completed ✅");

    Ok(())
}
