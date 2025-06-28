#![allow(clippy::too_many_arguments)]

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::jobs::metadata::{JobSpecificMetadata, SnosMetadata, StateUpdateMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
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
// Scenario 1: Block 0 is Completed | Block 1 is PendingRetry | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 2,3 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(0), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![], // missing_blocks_first_half (no missing in range [0, 0])
    vec![2, 3], // missing_blocks_second_half (blocks after 0)
    vec![1], // pending_blocks (block 1 is PendingRetry)
    vec![2, 3] // expected_jobs (skip pending block 1, create 2,3)
)]
// Scenario 2: Block 0 is Completed | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,2,3 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(0), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![], // missing_blocks_first_half (no missing in range [0, 0])
    vec![1, 2, 3], // missing_blocks_second_half (blocks after 0)
    vec![], // pending_blocks (no pending jobs)
    vec![1, 2, 3] // expected_jobs
)]
// Scenario 3: No SNOS job for any block exists | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 0,1,2 only
#[case(
    None,
    100,    // latest_sequencer_block
    None,   // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![], // missing_blocks_first_half (N/A when no completed SNOS)
    vec![0, 1, 2], // missing_blocks_second_half (start from min_block)
    vec![], // pending_blocks
    vec![0, 1, 2] // expected_jobs
)]
// Scenario 4: Block 0,2 is Completed | Block 1 is Missed | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,3,4 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(2), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![1], // missing_blocks_first_half (missing in range [0, 2])
    vec![3, 4], // missing_blocks_second_half (blocks after 2)
    vec![], // pending_blocks
    vec![1, 3, 4] // expected_jobs (fill missing first, then new blocks)
)]
// Scenario 5: Block 2 is Completed | Block 0 is PendingRetry | Block 1 is Missed | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 1,3 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(2), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![1], // missing_blocks_first_half (missing 1 in range [0, 2], 0 is pending)
    vec![3], // missing_blocks_second_half (blocks after 2)
    vec![0], // pending_blocks (block 0 is PendingRetry, consumes 1 slot)
    vec![1, 3] // expected_jobs (fill missing 1, then one new block due to slot constraint)
)]
// Scenario 6: Block 2 is Completed | Block 0 is PendingRetry | Block 1 is Created | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 3 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(2), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![], // missing_blocks_first_half (no missing blocks to create, 0 pending, 1 created)
    vec![3], // missing_blocks_second_half (blocks after 2)
    vec![0, 1], // pending_blocks (block 0 PendingRetry, block 1 Created, consume 2 slots)
    vec![3] // expected_jobs (only 1 slot left for new block)
)]
// Scenario 7: Block 4 is Created | latest_snos_completed & latest_state_transition_completed is 3  | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 3 only
#[case(
    None,
    100,    // latest_sequencer_block
    Some(3), // latest_snos_completed
    Some(3),   // latest_state_transition_completed
    vec![], // missing_blocks_first_half (no missing blocks to create)
    vec![5,6], // missing_blocks_second_half (no missing blocks to create)
    vec![4], // pending_blocks (block 4 Created, consumes 1 slot)
    vec![5,6] // expected_jobs (only 1 slot left for new block)
)]
// Scenario 8: Block 1 is Created | latest_snos_completed is 2 & latest_state_transition_completed is None | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 3 only
#[case(
    None,
    3,      // latest_sequencer_block
    Some(2), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![0], // missing_blocks_first_half (no missing blocks to create)
    vec![3], // missing_blocks_second_half
    vec![1], // pending_blocks (block 1 Created, consumes 1 slot)
    vec![0,3] // expected_jobs (only 2 slot left for new block)
)]
// Scenario 9: Block 1 is Created | earliest_failed_block is 4 | latest_snos_completed is 2 & latest_state_transition_completed is None | Max_concurrent_create_snos is 3
// Expected result: create jobs for block 3 only
#[case(
    Some(4), // earliest_failed_block
    5,      // latest_sequencer_block
    Some(2), // latest_snos_completed
    None,   // latest_state_transition_completed
    vec![0], // missing_blocks_first_half (no missing blocks to create)
    vec![3], // missing_blocks_second_half
    vec![1], // pending_blocks (block 1 Created, consumes 1 slot)
    vec![0,3] // expected_jobs (only 2 slot left for new block)
)]
#[tokio::test]
async fn test_snos_worker(
    #[case] earliest_failed_block: Option<u64>,
    #[case] latest_sequencer_block: u64,
    #[case] latest_snos_completed: Option<u64>,
    #[case] latest_state_transition_completed: Option<u64>,
    #[case] missing_blocks_first_half: Vec<u64>,
    #[case] missing_blocks_second_half: Vec<u64>,
    #[case] pending_blocks: Vec<u64>,
    #[case] expected_jobs_to_create: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");
    let min_block_limit = get_env_var_or_panic("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS").parse::<u64>()?;

    // Setup mock server and clients
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let mut job_handler = MockJobHandlerTrait::new();

    // Mock sequencer response
    let sequencer_response = json!({
        "id": 1,
        "jsonrpc": "2.0",
        "result": latest_sequencer_block
    });
    server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&sequencer_response).unwrap());
    });

    // Mock latest SNOS job
    let latest_snos_job = latest_snos_completed.map(|block_num| {
        let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
        job_item.metadata.specific =
            JobSpecificMetadata::Snos(SnosMetadata { block_number: block_num, ..Default::default() });
        job_item.status = JobStatus::Completed;
        job_item
    });

    db.expect_get_latest_job_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(latest_snos_job.clone()));

    db.expect_get_earliest_failed_block_number().with().returning(move || Ok(earliest_failed_block));

    // Mock get_job_by_internal_id_and_type to always return None
    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    // Mock latest StateTransition job
    let latest_state_transition_job = latest_state_transition_completed.map(|max_block| {
        let mut job_item = get_job_item_mock_by_id("state_transition".to_string(), Uuid::new_v4());
        let blocks_to_settle: Vec<u64> = (0..=max_block).collect();
        job_item.metadata.specific =
            JobSpecificMetadata::StateUpdate(StateUpdateMetadata { blocks_to_settle, ..Default::default() });
        job_item.status = JobStatus::Completed;
        job_item
    });

    db.expect_get_latest_job_by_type_and_status()
        .with(eq(JobType::StateTransition), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(latest_state_transition_job.clone()));

    // Mock pending jobs
    let pending_job_items: Vec<_> = pending_blocks
        .iter()
        .map(|block_num| {
            let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
            job_item.status = if block_num % 2 == 0 { JobStatus::PendingRetry } else { JobStatus::Created };
            job_item
        })
        .collect();

    db.expect_get_jobs_by_types_and_statuses()
        .with(eq(vec![JobType::SnosRun]), eq(vec![JobStatus::PendingRetry, JobStatus::Created]), eq(Some(3_i64)))
        .returning(move |_, _, _| Ok(pending_job_items.clone()));

    // Mock missing block number queries
    let missing_first_clone = missing_blocks_first_half.clone();
    let missing_second_clone = missing_blocks_second_half.clone();
    let latest_snos_clone = latest_snos_completed;
    let latest_state_clone = latest_state_transition_completed;

    let call_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let call_counter_clone = Arc::clone(&call_counter);

    db.expect_get_missing_block_numbers_by_type_and_caps()
        .withf(move |job_type, _, _, _| *job_type == JobType::SnosRun)
        .times(0..=2) // May be called 0, 1, or 2 times depending on the scenario
        .returning(move |_, lower, upper, _| {
            let call_num = call_counter_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let lower_bound = lower;
            let upper_bound = upper;

            // Determine which call this is based on the parameters
            if latest_snos_clone.is_none() {
                // When no SNOS jobs exist, only one call is made
                return Ok(missing_second_clone.clone());
            }

            if let Some(middle_limit) = latest_snos_clone {
                let actual_lower_limit = match latest_state_clone {
                    None => min_block_limit,
                    Some(state_block) => std::cmp::max(min_block_limit, state_block),
                };

                // First call: check for missing blocks in first half [lower_limit, middle_limit]
                if lower_bound == actual_lower_limit && upper_bound == middle_limit && call_num == 0 {
                    return Ok(missing_first_clone.clone());
                }

                // Second call: get new blocks in second half [middle_limit, upper_limit]
                if lower_bound == middle_limit && call_num <= 1 {
                    return Ok(missing_second_clone.clone());
                }
            }

            // Fallback
            Ok(missing_second_clone.clone())
        });

    // Mock job creation
    for &block_num in &expected_jobs_to_create {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();
        let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
        let job_item_clone = job_item.clone();

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
        .times(expected_jobs_to_create.len())
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

    println!(
        "✅ Test completed for scenario: latest_snos={:?}, pending_blocks={:?}, expected_jobs={:?}",
        latest_snos_completed, pending_blocks, expected_jobs_to_create
    );

    Ok(())
}
