pub mod constants;

use std::sync::Arc;

use crate::core::client::{MongoDbClient, AWSS3};
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
    let topic_name = aws_sns_params.endpoint.split(":").last().unwrap();
    let sns_client = get_sns_client(provider_config.get_aws_client_or_panic()).await;
    sns_client.create_topic().name(topic_name).send().await?;
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
    let bucket_name = s3_params.bucket_name.clone();
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

    // Dropping sqs queues
    let list_queues_output = sqs_client.list_queues().send().await?;
    let queue_urls = list_queues_output.queue_urls();
    tracing::debug!("Found {} queues", queue_urls.len());
    for queue_url in queue_urls {
        match sqs_client.delete_queue().queue_url(queue_url).send().await {
            Ok(_) => tracing::debug!("Successfully deleted queue: {}", queue_url),
            Err(e) => tracing::error!("Error deleting queue {}: {:?}", queue_url, e),
        }
    }

    for queue_type in QueueType::iter() {
        let queue_name = format!("{}_{}_{}", queue_params.prefix, queue_type, queue_params.suffix);
        sqs_client.create_queue().queue_name(queue_name).send().await?;
    }
    Ok(())
}

pub async fn get_sqs_client(provider_config: Arc<CloudProvider>) -> aws_sdk_sqs::Client {
    // This function is for localstack. So we can hardcode the region for this as of now.
    let config = provider_config.get_aws_client_or_panic();
    aws_sdk_sqs::Client::new(config)
}

#[derive(Deserialize, Debug)]
pub struct MessagePayloadType {
    pub(crate) id: Uuid,
}

pub async fn get_storage_client(provider_config: Arc<CloudProvider>) -> AWSS3 {
    AWSS3::create_setup(provider_config).await.unwrap()
}
