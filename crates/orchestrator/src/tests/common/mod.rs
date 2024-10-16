pub mod constants;

use std::collections::HashMap;
use std::sync::Arc;

use ::uuid::Uuid;
use aws_config::SdkConfig;
use aws_sdk_sns::error::SdkError;
use aws_sdk_sns::operation::create_topic::CreateTopicError;
use chrono::{SubsecRound, Utc};
use mongodb::Client;
use rstest::*;
use serde::Deserialize;
use utils::env_utils::get_env_var_or_panic;
use utils::settings::env::EnvSettingsProvider;

use crate::config::ProviderConfig;
use crate::data_storage::aws_s3::AWSS3;
use crate::data_storage::DataStorage;
use crate::database::mongodb::MongoDb;
use crate::jobs::types::JobStatus::Created;
use crate::jobs::types::JobType::DataSubmission;
use crate::jobs::types::{ExternalId, JobItem};
use crate::queue::job_queue::{
    DATA_SUBMISSION_JOB_PROCESSING_QUEUE, DATA_SUBMISSION_JOB_VERIFICATION_QUEUE,
    PROOF_REGISTRATION_JOB_PROCESSING_QUEUE, PROOF_REGISTRATION_JOB_VERIFICATION_QUEUE, PROVING_JOB_PROCESSING_QUEUE,
    PROVING_JOB_VERIFICATION_QUEUE, SNOS_JOB_PROCESSING_QUEUE, SNOS_JOB_VERIFICATION_QUEUE,
    UPDATE_STATE_JOB_PROCESSING_QUEUE, UPDATE_STATE_JOB_VERIFICATION_QUEUE,
};

#[fixture]
pub fn default_job_item() -> JobItem {
    JobItem {
        id: Uuid::new_v4(),
        internal_id: String::from("0"),
        job_type: DataSubmission,
        status: Created,
        external_id: ExternalId::String("0".to_string().into_boxed_str()),
        metadata: HashMap::new(),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}

#[fixture]
pub fn custom_job_item(default_job_item: JobItem, #[default(String::from("0"))] internal_id: String) -> JobItem {
    let mut job_item = default_job_item;
    job_item.internal_id = internal_id;

    job_item
}

pub async fn create_sns_arn(provider_config: Arc<ProviderConfig>) -> Result<(), SdkError<CreateTopicError>> {
    let sns_client = get_sns_client(provider_config.get_aws_client_or_panic()).await;
    sns_client.create_topic().name(get_env_var_or_panic("AWS_SNS_ARN_NAME")).send().await?;
    Ok(())
}

pub async fn get_sns_client(aws_config: &SdkConfig) -> aws_sdk_sns::client::Client {
    aws_sdk_sns::Client::new(aws_config)
}

pub async fn drop_database() -> color_eyre::Result<()> {
    let db_client: Client = MongoDb::new_with_settings(&EnvSettingsProvider {}).await.client();
    // dropping all the collection.
    // use .collection::<JobItem>("<collection_name>")
    // if only particular collection is to be dropped
    db_client.database("orchestrator").drop(None).await?;
    Ok(())
}

// SQS structs & functions

pub async fn create_sqs_queues(provider_config: Arc<ProviderConfig>) -> color_eyre::Result<()> {
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

    // Creating SQS queues
    sqs_client.create_queue().queue_name(DATA_SUBMISSION_JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(DATA_SUBMISSION_JOB_VERIFICATION_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(SNOS_JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(SNOS_JOB_VERIFICATION_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(PROVING_JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(PROVING_JOB_VERIFICATION_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(PROOF_REGISTRATION_JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(PROOF_REGISTRATION_JOB_VERIFICATION_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(UPDATE_STATE_JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(UPDATE_STATE_JOB_VERIFICATION_QUEUE).send().await?;

    Ok(())
}

pub async fn get_sqs_client(provider_config: Arc<ProviderConfig>) -> aws_sdk_sqs::Client {
    // This function is for localstack. So we can hardcode the region for this as of now.
    let config = provider_config.get_aws_client_or_panic();
    aws_sdk_sqs::Client::new(config)
}

#[derive(Deserialize, Debug)]
pub struct MessagePayloadType {
    pub(crate) id: Uuid,
}

pub async fn get_storage_client(provider_config: Arc<ProviderConfig>) -> Box<dyn DataStorage + Send + Sync> {
    Box::new(AWSS3::new_with_settings(&EnvSettingsProvider {}, provider_config).await)
}
