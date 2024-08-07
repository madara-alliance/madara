pub mod constants;

use std::collections::HashMap;
use std::sync::Arc;

use ::uuid::Uuid;
use aws_config::Region;
use constants::*;
use da_client_interface::MockDaClient;
use mongodb::Client;
use prover_client_interface::MockProverClient;
use rstest::*;
use serde::Deserialize;
use settlement_client_interface::MockSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

use crate::config::Config;
use crate::data_storage::aws_s3::config::{AWSS3ConfigType, S3LocalStackConfig};
use crate::data_storage::aws_s3::AWSS3;
use crate::data_storage::{DataStorage, DataStorageConfig, MockDataStorage};
use crate::database::mongodb::config::MongoDbConfig;
use crate::database::mongodb::MongoDb;
use crate::database::{DatabaseConfig, MockDatabase};
use crate::jobs::types::JobStatus::Created;
use crate::jobs::types::JobType::DataSubmission;
use crate::jobs::types::{ExternalId, JobItem};
use crate::queue::job_queue::{JOB_PROCESSING_QUEUE, JOB_VERIFICATION_QUEUE};
use crate::queue::MockQueueProvider;

pub async fn init_config(
    rpc_url: Option<String>,
    database: Option<MockDatabase>,
    queue: Option<MockQueueProvider>,
    da_client: Option<MockDaClient>,
    prover_client: Option<MockProverClient>,
    settlement_client: Option<MockSettlementClient>,
    storage_client: Option<MockDataStorage>,
) -> Config {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    let rpc_url = rpc_url.unwrap_or(MADARA_RPC_URL.to_string());
    let database = database.unwrap_or_default();
    let queue = queue.unwrap_or_default();
    let da_client = da_client.unwrap_or_default();
    let prover_client = prover_client.unwrap_or_default();
    let settlement_client = settlement_client.unwrap_or_default();
    let storage_client = storage_client.unwrap_or_default();

    // init starknet client
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(rpc_url.as_str()).expect("Failed to parse URL")));

    Config::new(
        Arc::new(provider),
        Box::new(da_client),
        Box::new(prover_client),
        Box::new(settlement_client),
        Box::new(database),
        Box::new(queue),
        Box::new(storage_client),
    )
}

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
    }
}

#[fixture]
pub fn custom_job_item(default_job_item: JobItem, #[default(String::from("0"))] internal_id: String) -> JobItem {
    let mut job_item = default_job_item;
    job_item.internal_id = internal_id;

    job_item
}

pub async fn drop_database() -> color_eyre::Result<()> {
    let db_client: Client = MongoDb::new(MongoDbConfig::new_from_env()).await.client();
    // dropping all the collection.
    // use .collection::<JobItem>("<collection_name>")
    // if only particular collection is to be dropped
    db_client.database("orchestrator").drop(None).await?;
    Ok(())
}

// SQS structs & functions

pub async fn create_sqs_queues() -> color_eyre::Result<()> {
    let sqs_client = get_sqs_client().await;

    // Dropping sqs queues
    let list_queues_output = sqs_client.list_queues().send().await?;
    let queue_urls = list_queues_output.queue_urls();
    log::debug!("Found {} queues", queue_urls.len());
    for queue_url in queue_urls {
        match sqs_client.delete_queue().queue_url(queue_url).send().await {
            Ok(_) => log::debug!("Successfully deleted queue: {}", queue_url),
            Err(e) => eprintln!("Error deleting queue {}: {:?}", queue_url, e),
        }
    }

    // Creating SQS queues
    sqs_client.create_queue().queue_name(JOB_PROCESSING_QUEUE).send().await?;
    sqs_client.create_queue().queue_name(JOB_VERIFICATION_QUEUE).send().await?;
    Ok(())
}

async fn get_sqs_client() -> aws_sdk_sqs::Client {
    // This function is for localstack. So we can hardcode the region for this as of now.
    let region_provider = Region::new("us-east-1");
    let config = aws_config::from_env().region(region_provider).load().await;
    aws_sdk_sqs::Client::new(&config)
}

#[derive(Deserialize, Debug)]
pub struct MessagePayloadType {
    pub(crate) id: Uuid,
}

pub async fn get_storage_client() -> Box<dyn DataStorage + Send + Sync> {
    Box::new(AWSS3::new(AWSS3ConfigType::WithEndpoint(S3LocalStackConfig::new_from_env())).await)
}
