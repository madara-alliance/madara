pub mod constants;

use std::collections::HashMap;
use std::sync::Arc;

use ::uuid::Uuid;
use constants::*;
use da_client_interface::MockDaClient;
use prover_client_interface::MockProverClient;
use rstest::*;
use settlement_client_interface::MockSettlementClient;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

use crate::config::Config;
use crate::database::MockDatabase;
use crate::jobs::types::JobStatus::Created;
use crate::jobs::types::JobType::DataSubmission;
use crate::jobs::types::{ExternalId, JobItem};
use crate::queue::MockQueueProvider;

pub async fn init_config(
    rpc_url: Option<String>,
    database: Option<MockDatabase>,
    queue: Option<MockQueueProvider>,
    da_client: Option<MockDaClient>,
    prover_client: Option<MockProverClient>,
    settlement_client: Option<MockSettlementClient>,
) -> Config {
    let _ = tracing_subscriber::fmt().with_max_level(tracing::Level::INFO).with_target(false).try_init();

    let rpc_url = rpc_url.unwrap_or(MADARA_RPC_URL.to_string());
    let database = database.unwrap_or_default();
    let queue = queue.unwrap_or_default();
    let da_client = da_client.unwrap_or_default();
    let prover_client = prover_client.unwrap_or_default();
    let settlement_client = settlement_client.unwrap_or_default();

    // init starknet client
    let provider = JsonRpcClient::new(HttpTransport::new(Url::parse(rpc_url.as_str()).expect("Failed to parse URL")));

    Config::new(
        Arc::new(provider),
        Box::new(da_client),
        Box::new(prover_client),
        Box::new(settlement_client),
        Box::new(database),
        Box::new(queue),
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
