use std::process::Command;
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
    sleep(Duration::from_secs(100)).await;
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

pub async fn setup_db() -> color_eyre::Result<()> {
    // We run the js script in the folder root:
    println!("Setting up database.");

    Command::new("node").arg("migrate-mongo-config.js").output()?;

    println!("Database setup completed ✅");

    Ok(())
}
