pub mod constants;
#[cfg(test)]
pub mod mock_helpers;

use crate::core::client::queue::sqs::InnerSQS;
use crate::core::client::queue::QueueClient;
use crate::core::client::MongoDbClient;
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::Duration;

use crate::core::client::storage::s3::InnerAWSS3;
use crate::core::cloud::CloudProvider;
use crate::core::traits::resource::Resource;
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::types::JobStatus::Created;
use crate::types::jobs::types::JobType::DataSubmission;
use crate::types::params::database::DatabaseArgs;
use crate::types::params::{AlertArgs, QueueArgs, StorageArgs};
use crate::types::queue::QueueType;
use ::uuid::Uuid;
use aws_config::SdkConfig;
use aws_sdk_s3::Client as S3Client;
use aws_sdk_sns::error::SdkError;
use aws_sdk_sns::operation::create_topic::CreateTopicError;
use chrono::{SubsecRound, Utc};
use mongodb::Client;
use rstest::*;
use serde::Deserialize;
use strum::IntoEnumIterator as _;

#[fixture]
pub fn default_job_item() -> JobItem {
    JobItem {
        id: Uuid::new_v4(),
        internal_id: String::from("0"),
        job_type: DataSubmission,
        status: Created,
        external_id: ExternalId::String("0".to_string().into_boxed_str()),
        metadata: JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Da(DaMetadata { block_number: 0, blob_data_path: None, tx_hash: None }),
        },
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

#[fixture]
pub fn custom_job_item(default_job_item: JobItem, #[default(String::from("0"))] internal_id: String) -> JobItem {
    let mut job_item = default_job_item;
    job_item.internal_id = internal_id.clone();

    // Update block number in metadata to match internal_id if possible
    if let Ok(block_number) = internal_id.parse::<u64>() {
        if let JobSpecificMetadata::Da(ref mut da_metadata) = job_item.metadata.specific {
            da_metadata.block_number = block_number;
        }
    }

    job_item
}

pub async fn create_sns_arn(
    provider_config: Arc<CloudProvider>,
    aws_sns_params: &AlertArgs,
) -> Result<(), SdkError<CreateTopicError>> {
    let alert_topic_name = aws_sns_params.alert_identifier.to_string();
    let sns_client = get_sns_client(provider_config.get_aws_client_or_panic()).await;
    sns_client.create_topic().name(alert_topic_name).send().await?;
    Ok(())
}

pub async fn get_sns_client(aws_config: &SdkConfig) -> aws_sdk_sns::client::Client {
    aws_sdk_sns::Client::new(aws_config)
}

pub async fn drop_database(mongodb_params: &DatabaseArgs) -> color_eyre::Result<()> {
    let db_client: Client = MongoDbClient::new(mongodb_params).await?.client();
    db_client.database(&mongodb_params.database_name).drop(None).await?;
    Ok(())
}

pub async fn delete_storage(provider_config: Arc<CloudProvider>, s3_params: &StorageArgs) -> color_eyre::Result<()> {
    let bucket_name = s3_params.bucket_identifier.to_string();
    let aws_config = provider_config.get_aws_client_or_panic();

    let mut s3_config_builder = aws_sdk_s3::config::Builder::from(aws_config);
    // this is necessary for it to work with localstack in test cases
    s3_config_builder.set_force_path_style(Some(true));
    let client = S3Client::from_conf(s3_config_builder.build());
    // Check if bucket exists
    match client.head_bucket().bucket(&bucket_name).send().await {
        Ok(_) => {
            println!("Bucket exists, proceeding with deletion");
        }
        Err(e) => {
            println!("Bucket '{}' does not exist or is not accessible: {}", &bucket_name, e);
            return Ok(());
        }
    }

    // First, delete all objects in the bucket (required for non-empty buckets)
    let objects = client.list_objects_v2().bucket(&bucket_name).send().await?.contents().to_vec();

    // If there are objects, delete them
    if !objects.is_empty() {
        let objects_to_delete: Vec<_> = objects
            .iter()
            .map(|obj| {
                aws_sdk_s3::types::ObjectIdentifier::builder()
                    .key(obj.key().unwrap_or_default())
                    .build()
                    .expect("Could not build object builder")
            })
            .collect();

        let delete = aws_sdk_s3::types::Delete::builder().set_objects(Some(objects_to_delete)).build()?;

        client.delete_objects().bucket(&bucket_name).delete(delete).send().await?;
    }

    // After deleting all objects, delete the bucket itself
    client.delete_bucket().bucket(&bucket_name).send().await?;

    Ok(())
}

// SQS structs & functions

pub async fn create_queues(provider_config: Arc<CloudProvider>, queue_params: &QueueArgs) -> color_eyre::Result<()> {
    let sqs_client = get_sqs_client(provider_config).await;

    // Get the queue template identifier for this test
    let queue_template = queue_params.queue_template_identifier.to_string();

    // Create all queues for this test
    // Note: We don't delete existing queues since:
    // 1. Each test has a unique UUID, so queues are isolated
    // 2. LocalStack will be killed after tests, so cleanup isn't needed
    // 3. Deleting queues can cause race conditions in parallel execution
    for queue_type in QueueType::iter() {
        let queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &queue_type);

        // Create queue, handling the case where it might already exist
        match sqs_client.create_queue().queue_name(queue_name.clone()).send().await {
            Ok(_) => tracing::debug!("Created queue: {}", queue_name),
            Err(e) => {
                // If queue already exists, that's okay - we'll use the existing one
                let error_str = e.to_string();
                if error_str.contains("QueueAlreadyExists") || error_str.contains("QueueNameExists") {
                    tracing::debug!("Queue {} already exists, using existing queue", queue_name);
                } else {
                    // For other errors, return the error
                    tracing::error!("Failed to create queue {}: {:?}", queue_name, e);
                    return Err(e.into());
                }
            }
        }
    }

    // Wait longer for queues to be fully available (LocalStack sometimes needs more time)
    // This ensures queues are ready before tests try to use them
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    tracing::debug!("All queues created and ready for test");
    Ok(())
}

pub async fn get_sqs_client(provider_config: Arc<CloudProvider>) -> aws_sdk_sqs::Client {
    // This function is for localstack. So we can hardcode the region for this as of now.
    let config = provider_config.get_aws_client_or_panic();
    aws_sdk_sqs::Client::new(config)
}

/// Cleanup queues for a specific test (only deletes queues matching the queue_params identifier)
pub async fn cleanup_queues(provider_config: Arc<CloudProvider>, queue_params: &QueueArgs) -> color_eyre::Result<()> {
    let sqs_client = get_sqs_client(provider_config).await;
    let queue_template = queue_params.queue_template_identifier.to_string();

    // Delete only queues belonging to this test
    for queue_type in QueueType::iter() {
        let queue_name = InnerSQS::get_queue_name_from_type(&queue_template, &queue_type);

        match sqs_client.get_queue_url().queue_name(&queue_name).send().await {
            Ok(output) => {
                if let Some(queue_url) = output.queue_url() {
                    match sqs_client.delete_queue().queue_url(queue_url).send().await {
                        Ok(_) => tracing::debug!("Deleted queue: {}", queue_name),
                        Err(e) => tracing::warn!("Error deleting queue {}: {:?}", queue_name, e),
                    }
                }
            }
            Err(_) => {
                tracing::debug!("Queue {} does not exist, skipping", queue_name);
            }
        }
    }

    Ok(())
}

#[derive(Deserialize, Debug)]
pub struct MessagePayloadType {
    pub(crate) id: Uuid,
}

pub(crate) async fn get_storage_client(provider_config: Arc<CloudProvider>) -> InnerAWSS3 {
    InnerAWSS3::create_setup(provider_config).await.unwrap()
}

/// Helper function to consume a message from a queue with retry logic.
/// This function handles common timing issues in test environments (e.g., LocalStack)
/// by retrying when queues don't exist yet or when messages aren't immediately available.
///
/// # Arguments
/// * `queue_client` - The queue client to use for consuming messages
/// * `queue_type` - The type of queue to consume from
/// * `max_retries` - Maximum number of retry attempts (default: 5)
/// * `retry_delay` - Delay between retries in seconds (default: 1)
///
/// # Returns
/// * `Delivery` - The consumed message
///
/// # Panics
/// * Panics if the queue doesn't exist after max_retries attempts
/// * Panics if no message is found after max_retries attempts
/// * Panics if any other error occurs after max_retries attempts
pub async fn consume_message_with_retry(
    queue_client: &dyn QueueClient,
    queue_type: QueueType,
    max_retries: usize,
    retry_delay: u64,
) -> Delivery {
    let mut retries = 0;
    let mut queue_message = None;

    while retries < max_retries && queue_message.is_none() {
        match queue_client.consume_message_from_queue(queue_type.clone()).await {
            Ok(msg) => queue_message = Some(msg),
            Err(e) => {
                let error_str = e.to_string();
                if error_str.contains("QueueDoesNotExist") || error_str.contains("GetQueueUrlError") {
                    if retries < max_retries - 1 {
                        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
                        retries += 1;
                    } else {
                        panic!("Queue does not exist after {} retries: {}", max_retries, e);
                    }
                } else if error_str.contains("NoData") {
                    if retries < max_retries - 1 {
                        tokio::time::sleep(Duration::from_secs(retry_delay)).await;
                        retries += 1;
                    } else {
                        panic!("No message found in queue after {} retries: {}", max_retries, e);
                    }
                } else if retries < max_retries - 1 {
                    tokio::time::sleep(Duration::from_secs(retry_delay)).await;
                    retries += 1;
                } else {
                    panic!("Failed to consume message: {}", e);
                }
            }
        }
    }

    queue_message.expect("Should have received message")
}
