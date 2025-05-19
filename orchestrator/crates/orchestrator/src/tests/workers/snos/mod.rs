use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::error::Error;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

#[rstest]
// Scenario 1:
// Block 0 is Completed | Block 1 is PendingRetry | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 2,3 only
// test_scenario_1_block_0_completed_block_1_pending_retry
#[case(vec![0], vec![1], vec![], vec![2,3])]
// Scenario 2:
// Block 0 is Completed | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,2,3 only
// test_scenario_2_block_0_completed_only
#[case(vec![0], vec![], vec![], vec![1,2,3])]
// Scenario 3:
// No SNOS job for any block exists | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 0,1,2 only
#[case(vec![], vec![], vec![], vec![0,1,2])]
// Scenario 4:
// Block 0,2 is Completed | Block 1 is Missed (should be created) | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,3,4 only
#[case(vec![0,2], vec![], vec![], vec![1,3,4])]
// Scenario 5:
// Block 2 is Completed | Block 0 is PendingRetry | Block 1 is Missed (should be created) | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,3 only
#[case(vec![2], vec![0], vec![], vec![1,3])]
// Scenario 6:
// Block 2 is Completed | Block 0 is PendingRetry | Block 1 is Created | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 3 only
#[case(vec![2], vec![0], vec![1], vec![3])]
#[tokio::test]
async fn test_snos_worker(
    #[case] completed_blocks: Vec<u64>,
    #[case] pending_retry_blocks: Vec<u64>,
    #[case] created_blocks: Vec<u64>,
    #[case] expected_jobs_to_create: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    // Mocking the get_job_handler function
    let mut job_handler = MockJobHandlerTrait::new();

    // 1. Mock get_jobs_by_type_and_status for Completed jobs
    let mut completed_job_items = Vec::new();
    for block_num in &completed_blocks {
        let uuid = Uuid::new_v4();
        completed_job_items.push(get_job_item_mock_by_id(block_num.to_string(), uuid));
    }
    db.expect_get_jobs_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(completed_job_items.clone()));

    // Mock get_job_by_internal_id_and_type to always return None
    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    // 2. Mock get_jobs_by_type_and_status for PendingRetry jobs
    let mut pending_job_items = Vec::new();
    for block_num in &pending_retry_blocks {
        let uuid = Uuid::new_v4();
        pending_job_items.push(get_job_item_mock_by_id(block_num.to_string(), uuid));
    }
    db.expect_get_jobs_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::PendingRetry))
        .returning(move |_, _| Ok(pending_job_items.clone()));

    // 3. Mock get_jobs_by_type_and_status for Created jobs
    let mut created_job_items = Vec::new();
    for block_num in &created_blocks {
        let uuid = Uuid::new_v4();
        created_job_items.push(get_job_item_mock_by_id(block_num.to_string(), uuid));
    }
    db.expect_get_jobs_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Created))
        .returning(move |_, _| Ok(created_job_items.clone()));

    // Setup job creation expectations for each expected job
    for &block_num in &expected_jobs_to_create {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();
        let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
        let job_item_clone = job_item.clone();

        job_handler
            .expect_create_job()
            .with(eq(block_num_str.clone()), mockall::predicate::always())
            .returning(move |_, _| Ok(job_item_clone.clone()));

        // Create a copy of block_num_str for the db closure
        let block_num_str_for_db = block_num_str.clone();
        db.expect_create_job()
            .withf(move |item| item.internal_id == block_num_str_for_db)
            .returning(move |_| Ok(job_item.clone()));
    }

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Queue function call simulations
    queue
        .expect_send_message()
        .times(expected_jobs_to_create.len())
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

    // Mock RPC response for block_number
    let response = json!({ "id": 1, "jsonrpc": "2.0", "result": 100 });

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    // Configure test environment
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Mock block_number RPC call
    let rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    // Run the worker
    crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await?;

    // Verify RPC call was made
    rpc_block_call_mock.assert();

    Ok(())
}
