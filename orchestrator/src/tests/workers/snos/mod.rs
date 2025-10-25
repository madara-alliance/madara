use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::batch::{SnosBatch, SnosBatchStatus};
use crate::types::jobs::types::JobType;
use crate::types::queue::QueueType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_utils::env_utils::get_env_var_or_panic;
use rstest::rstest;
use serde_json::json;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::error::Error;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

#[rstest]
#[case(vec![1, 2, 3])]
#[tokio::test]
async fn test_snos_worker(#[case] completed_snos_batches: Vec<u64>) -> Result<(), Box<dyn Error>> {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");

    // Setup mock server and clients
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let mut job_handler = MockJobHandlerTrait::new();

    db.expect_get_orphaned_jobs().returning(|_, _| Ok(Vec::new()));

    db.expect_get_snos_batches_without_jobs().with(eq(SnosBatchStatus::Closed)).returning({
        let completed_snos_batches = completed_snos_batches.clone();
        move |_| Ok(completed_snos_batches.iter().map(|&index| SnosBatch::new(index, 1, index)).collect())
    });

    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    db.expect_update_or_create_snos_batch().returning(|_, _| Ok(SnosBatch::new(1, 1, 1)));

    // Mock job creation
    for &block_num in &completed_snos_batches {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();
        let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
        let job_item_clone = job_item.clone();

        db.expect_get_snos_batches_by_indices()
            .withf({
                let snos_batch_num = block_num;
                move |i| *i == vec![snos_batch_num]
            })
            .times(1)
            .returning(|_| Ok(Vec::new()));

        job_handler
            .expect_create_job()
            .with(eq(block_num_str.clone()), mockall::predicate::always())
            .returning(move |_, _| Ok(job_item_clone.clone()));

        let block_num_str_for_db = block_num_str.clone();
        db.expect_create_job()
            .withf(move |item| item.internal_id == block_num_str_for_db)
            .returning(move |_| Ok(job_item.clone()));
    }

    // Setup job handler context
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Mock queue operations
    queue
        .expect_send_message()
        .times(completed_snos_batches.len())
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _, _| *queue == QueueType::SnosJobProcessing);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    // Build test configuration
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the SNOS worker
    crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await?;

    Ok(())
}

/// Test for creating a SNOS job using the SNOS job trigger with a pre-existing SNOS batch.
/// This test simulates the workflow where a SNOS batch is created and then
/// the SNOS job trigger's run_worker method creates a corresponding SNOS job.
#[rstest]
#[case(vec![1, 2, 3])]
#[tokio::test]
async fn test_create_snos_job_for_existing_batch(
    #[case] completed_snos_batches: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");
    let _min_block_limit = get_env_var_or_panic("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS").parse::<u64>()?;

    let num_snos_batches = completed_snos_batches.len();

    // Set up the mock server and clients
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let mut job_handler = MockJobHandlerTrait::new();

    // Mock sequencer response - set to a high block number
    let sequencer_response = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "result": 1000
    });
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&sequencer_response).unwrap());
    });

    // No orphaned jobs
    db.expect_get_orphaned_jobs().returning(|_, _| Ok(Vec::new()));

    // Getting the list of snos batches without jobs should return the completed snos batches
    db.expect_get_snos_batches_without_jobs().times(1).with(eq(SnosBatchStatus::Closed)).returning({
        let completed_snos_batches = completed_snos_batches.clone();
        move |_| Ok(completed_snos_batches.iter().map(|&index| SnosBatch::new(index, 1, index)).collect())
    });

    // This is called to check if we have a job with the same internal_id and type
    // Since we don't have any jobs, we should return None for all calls
    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    // Doesn't matter what we return here
    db.expect_update_or_create_snos_batch().returning(|_, _| Ok(SnosBatch::new(1, 1, 1)));

    // Mock job creation for our test batch
    for snos_batch_num in completed_snos_batches {
        let uuid = Uuid::new_v4();
        let job_item = get_job_item_mock_by_id(snos_batch_num.to_string(), uuid);
        let job_item_clone = job_item.clone();

        db.expect_get_snos_batches_by_indices()
            .withf(move |i| *i == vec![snos_batch_num])
            .times(1)
            .returning(|_| Ok(Vec::new()));

        db.expect_create_job()
            .withf(move |item| item.internal_id == snos_batch_num.to_string())
            .times(1)
            .returning(move |_| Ok(job_item.clone()));

        job_handler
            .expect_create_job()
            .with(eq(snos_batch_num.to_string()), mockall::predicate::always())
            .times(1)
            .returning(move |_, _| Ok(job_item_clone.clone()));
    }

    // Setup job handler context
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Expecting 3 calls to send_message for the SNOS job trigger
    queue
        .expect_send_message()
        .times(num_snos_batches)
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _, _| *queue == QueueType::SnosJobProcessing);

    // Setup provider
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    // Build test configuration
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the SNOS worker
    let result = crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await;

    // Verify the worker succeeded
    assert!(result.is_ok(), "SNOS job trigger run_worker should succeed");

    println!("✅ Test completed: SNOS job created for existing batch successfully");

    Ok(())
}
