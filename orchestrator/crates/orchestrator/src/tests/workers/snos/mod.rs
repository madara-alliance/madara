use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;

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
use url::Url;
use uuid::Uuid;

#[rstest]
#[case(800, 100, vec![100, 200, 300, 400], vec![123, 213, 341], vec![333, 111], vec![389, 390, 391, 392])]
#[case(800, 50, vec![100, 200, 300, 400], vec![67, 123, 213], vec![333], vec![389, 390, 391])]
#[tokio::test]
async fn test_snos_worker_with_missing_blocks(
    #[case] latest_block: u64,
    #[case] max_concurrent_jobs: u64,
    #[case] completed_blocks: Vec<u64>,
    #[case] missing_blocks: Vec<u64>,
    #[case] pending_retry_blocks: Vec<u64>,
    #[case] created_blocks: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    // Mocking the get_job_handler function
    let mut job_handler = MockJobHandlerTrait::new();

    // Calculate expected values
    let last_completed_block = *completed_blocks.iter().max().unwrap_or(&0);
    let total_pending_and_created = (pending_retry_blocks.len() + created_blocks.len()) as u64;
    let available_job_slots = max_concurrent_jobs - total_pending_and_created;

    // Build a set of all processed blocks (for later jobs_to_create calculation)
    let mut processed_blocks = HashSet::new();
    processed_blocks.extend(completed_blocks.iter());

    // Expected jobs_to_create calculation
    let mut expected_jobs = Vec::new();

    // First, add missing blocks (prioritized)
    let mut sorted_missing = missing_blocks.clone();
    sorted_missing.sort();
    let missing_blocks_to_add: Vec<u64> = sorted_missing.into_iter().take(available_job_slots as usize).collect();
    expected_jobs.extend(missing_blocks_to_add.iter());

    // Calculate remaining slots for new blocks
    let remaining_slots = available_job_slots - missing_blocks_to_add.len() as u64;

    // Add new blocks starting from last_completed_block + 1
    if remaining_slots > 0 {
        let start_block = last_completed_block + 1;
        let end_block = std::cmp::min(start_block + remaining_slots - 1, latest_block);
        expected_jobs.extend((start_block..=end_block).filter(|b| !processed_blocks.contains(b)));
    }

    // Sort all blocks
    expected_jobs.sort();

    // Mock database calls

    // 1. Mock get_jobs_by_type_and_status for Completed jobs
    let mut completed_job_items = Vec::new();
    for block_num in &completed_blocks {
        let uuid = Uuid::new_v4();
        completed_job_items.push(get_job_item_mock_by_id(block_num.to_string(), uuid));
    }
    db.expect_get_jobs_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(completed_job_items.clone()));

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
    for block_num in &expected_jobs {
        let uuid = Uuid::new_v4();
        let job_item = get_job_item_mock_by_id(block_num.to_string(), uuid);
        let job_item_clone = job_item.clone();

        job_handler
            .expect_create_job()
            .with(eq(block_num.to_string()), mockall::predicate::always())
            .returning(move |_, _| Ok(job_item_clone.clone()));

        db.expect_create_job()
            .withf(move |item| item.internal_id == block_num.to_string())
            .returning(move |_| Ok(job_item.clone()));
    }

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Queue function call simulations
    queue
        .expect_send_message()
        .times(expected_jobs.len())
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

    // Mock RPC response for block_number
    let response = json!({ "id": 1, "jsonrpc": "2.0", "result": latest_block });

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    // Configure test environment
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_service_config(|config| {
            config.max_block_to_process = Some(latest_block + 1000); // Set higher than latest_block
            config.min_block_to_process = Some(0);
            config.max_concurrent_created_snos_jobs = Some(max_concurrent_jobs as usize);
        })
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

#[rstest]
#[case(800, None, 400, vec![213, 341, 123], vec![], vec![])]
#[case(800, None, 400, vec![], vec![], vec![])]
#[tokio::test]
async fn test_snos_worker_unlimited_jobs(
    #[case] latest_block: u64,
    #[case] max_concurrent_jobs: Option<usize>,
    #[case] last_completed_block: u64,
    #[case] missing_blocks: Vec<u64>,
    #[case] pending_retry_blocks: Vec<u64>,
    #[case] created_blocks: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    // Mocking the get_job_handler function
    let mut job_handler = MockJobHandlerTrait::new();

    // Create completed job items
    let mut completed_job_items = Vec::new();
    let mut processed_blocks = HashSet::new();

    // Add the last_completed_block to completed_job_items
    let uuid_last = Uuid::new_v4();
    completed_job_items.push(get_job_item_mock_by_id(last_completed_block.to_string(), uuid_last));
    processed_blocks.insert(last_completed_block);

    // Add some other completed blocks
    for i in 1..last_completed_block {
        if !missing_blocks.contains(&i) {
            let uuid = Uuid::new_v4();
            completed_job_items.push(get_job_item_mock_by_id(i.to_string(), uuid));
            processed_blocks.insert(i);
        }
    }

    // Expected jobs to create
    let mut expected_jobs = Vec::new();

    // Add missing blocks first
    expected_jobs.extend(&missing_blocks);

    // Then add new blocks from last_completed_block+1 to latest_block
    expected_jobs.extend((last_completed_block + 1)..=latest_block);

    // Sort all expected jobs
    expected_jobs.sort();

    // 1. Mock get_jobs_by_type_and_status for Completed jobs
    db.expect_get_jobs_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(completed_job_items.clone()));

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
    for block_num in &expected_jobs {
        let uuid = Uuid::new_v4();
        let job_item = get_job_item_mock_by_id(block_num.to_string(), uuid);
        let job_item_clone = job_item.clone();

        job_handler
            .expect_create_job()
            .with(eq(block_num.to_string()), mockall::predicate::always())
            .returning(move |_, _| Ok(job_item_clone.clone()));

        db.expect_create_job()
            .withf(move |item| item.internal_id == block_num.to_string())
            .returning(move |_| Ok(job_item.clone()));
    }

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Queue function call simulations
    queue
        .expect_send_message()
        .times(expected_jobs.len())
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

    // Mock RPC response for block_number
    let response = json!({ "id": 1, "jsonrpc": "2.0", "result": latest_block });

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    // Configure test environment
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_service_config(|config| {
            config.max_block_to_process = Some(latest_block + 1000); // Set higher than latest_block
            config.min_block_to_process = Some(0);
            config.max_concurrent_created_snos_jobs = max_concurrent_jobs;
        })
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
