pub mod constants;

use constants::*;
use rstest::*;

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    config::Config,
    jobs::types::{ExternalId, JobItem, JobStatus::Created, JobType::DataSubmission},
};

use crate::database::MockDatabase;
use crate::queue::MockQueueProvider;
use ::uuid::Uuid;
use da_client_interface::MockDaClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;

use url::Url;

pub async fn init_config(
    rpc_url: Option<String>,
    database: Option<MockDatabase>,
    queue: Option<MockQueueProvider>,
    da_client: Option<MockDaClient>,
) -> Config {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    let rpc_url = rpc_url.unwrap_or(MADARA_RPC_URL.to_string());
    let database = database.unwrap_or_default();
    let queue = queue.unwrap_or_default();
    let da_client = da_client.unwrap_or_default();

    // init starknet client
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(rpc_url.as_str()).expect("Failed to parse URL")));

    Config::new(Arc::new(provider), Box::new(da_client), Box::new(database), Box::new(queue))
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
