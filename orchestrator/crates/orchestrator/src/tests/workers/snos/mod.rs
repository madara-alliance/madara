use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata, StateUpdateMetadata};
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
use std::env;
use std::error::Error;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

#[rstest]
// Scenario 1: Fresh start - no completed jobs, should create jobs from min_block
// State: sequencer_block: 1000, min: 0, max: 999, slots: 5
// No SNOS completed, No StateUpdate completed, No pending jobs
// Expected: Create jobs [0, 1, 2, 3, 4] (min_block to min_block + slots - 1)
#[case(1000, 0, 999, 5, None, None, vec![], vec![], vec![], vec![0, 1, 2, 3, 4])]
// Scenario 2: Normal progression - SNOS completed up to block 10, create next batch
// State: sequencer_block: 1000, min: 0, max: 999, slots: 3
// SNOS completed: 10, No StateUpdate, No missing blocks, No pending
// Expected: Create jobs [11, 12, 13] (next consecutive blocks after last completed)
#[case(1000, 0, 999, 3, Some(10), None, vec![], vec![], vec![], vec![11, 12, 13])]
// Scenario 3: Fill missing blocks first, then create new ones
// State: sequencer_block: 1000, min: 0, max: 999, slots: 4
// SNOS completed: 10, Missing blocks: [3, 7], No pending
// Expected: Create jobs [3, 7, 11, 12] (missing blocks first, then new ones)
#[case(1000, 0, 999, 4, Some(10), None, vec![3, 7], vec![], vec![], vec![3, 7, 11, 12])]
// Scenario 4: StateUpdate constraint affects lower bound
// State: sequencer_block: 1000, min: 0, max: 999, slots: 3
// SNOS completed: 5, StateUpdate completed: 8, No missing, No pending
// Expected: Create jobs [9, 10, 11] (start from max(StateUpdate+1, SNOS+1))
#[case(1000, 0, 999, 3, Some(5), Some(8), vec![], vec![], vec![], vec![9, 10, 11])]
// Scenario 5: Pending jobs consume available slots
// State: sequencer_block: 1000, min: 0, max: 999, slots: 4
// SNOS completed: 10, Pending jobs: [11, 12] (2 slots used)
// Expected: Create jobs [13, 14] (only 2 slots available after pending)
#[case(1000, 0, 999, 4, Some(10), None, vec![], vec![11, 12], vec![], vec![13, 14])]
// Scenario 6: All slots occupied by pending jobs - no new jobs created
// State: sequencer_block: 1000, min: 0, max: 999, slots: 2
// SNOS completed: 10, Pending jobs: [11, 12] (all slots occupied)
// Expected: Create no jobs [] (no available slots)
#[case(1000, 0, 999, 2, Some(10), None, vec![], vec![11, 12], vec![], vec![])]
// Scenario 7: Upper limit constraint from max_block_to_process
// State: sequencer_block: 1000, min: 0, max: 12, slots: 5
// SNOS completed: 10, No pending
// Expected: Create jobs [11, 12] (limited by max_block_to_process)
#[case(1000, 0, 12, 5, Some(10), None, vec![], vec![], vec![], vec![11, 12])]
// Scenario 8: Upper limit constraint from sequencer block number
// State: sequencer_block: 13, min: 0, max: 999, slots: 5
// SNOS completed: 10, No pending
// Expected: Create jobs [11, 12, 13] (limited by sequencer block)
#[case(13, 0, 999, 5, Some(10), None, vec![], vec![], vec![], vec![11, 12, 13])]
// Scenario 9: Complex scenario with missing blocks across ranges and constraints
// State: sequencer_block: 1000, min: 5, max: 999, slots: 6
// SNOS completed: 15, StateUpdate completed: 8, Missing: [6, 9, 12], Pending: [16]
// Lower limit = max(8, 5) = 8, Upper limit = min(1000, 999) = 999
// Missing in [8, 15]: [9, 12] (2 blocks), Pending: 1 slot used
// Remaining slots: 6 - 1 = 5, after missing: 5 - 2 = 3
// Expected: Create jobs [9, 12, 17, 18, 19] (missing first, then new after pending)
#[case(1000, 5, 999, 6, Some(15), Some(8), vec![6, 9, 12], vec![16], vec![], vec![9, 12, 17, 18, 19])]
// Scenario 10: Edge case - SNOS completed at 0, should handle correctly
// State: sequencer_block: 1000, min: 0, max: 999, slots: 3
// SNOS completed: 0, No StateUpdate, No missing, No pending
// Expected: Create jobs [1, 2, 3] (next blocks after 0)
#[case(1000, 0, 999, 3, Some(0), None, vec![], vec![], vec![], vec![1, 2, 3])]
// Scenario 11: Min block constraint higher than completed blocks
// State: sequencer_block: 1000, min: 20, max: 999, slots: 3
// SNOS completed: 15, No StateUpdate, No missing, No pending
// Lower limit = max(20, min) = 20, Upper limit = min(1000, 999) = 999
// Expected: Create jobs [20, 21, 22] (start from min_block constraint)
#[case(1000, 20, 999, 3, Some(15), None, vec![], vec![], vec![], vec![20, 21, 22])]
#[tokio::test]
async fn test_snos_trigger_scenarios(
    #[case] sequencer_latest_block: u64,
    #[case] min_block_to_process: u64,
    #[case] max_block_to_process: u64,
    #[case] max_concurrent_jobs: u64,
    #[case] latest_snos_completed_block: Option<u64>,
    #[case] latest_state_transition_block: Option<u64>,
    #[case] missing_blocks: Vec<u64>,
    #[case] pending_blocks: Vec<u64>,
    #[case] expected_blocks: Vec<u64>,
    #[case] expected_jobs_to_create: Vec<u64>,
) -> Result<(), Box<dyn Error>> {
    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let mut job_handler = MockJobHandlerTrait::new();

    // Mock sequencer RPC call
    let sequencer_response = json!({ "id": 1, "jsonrpc": "2.0", "result": sequencer_latest_block });
    let _rpc_block_call_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_blockNumber");
        then.status(200).body(serde_json::to_vec(&sequencer_response).unwrap());
    });

    // Mock latest SNOS job
    let latest_snos_job = latest_snos_completed_block.map(|block_num| {
        let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
        job_item.metadata.specific =
            JobSpecificMetadata::Snos(SnosMetadata { block_number: block_num, ..Default::default() });
        job_item.status = JobStatus::Completed;
        job_item
    });

    db.expect_get_latest_job_by_type_and_status()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
        .returning(move |_, _| Ok(latest_snos_job.clone()));

    // Mock latest StateTransition job
    let latest_state_transition_job = latest_state_transition_block.map(|max_block| {
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
        .map(|&block_num| {
            let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
            job_item.status = JobStatus::PendingRetry;
            job_item.metadata.specific =
                JobSpecificMetadata::Snos(SnosMetadata { block_number: block_num, ..Default::default() });
            job_item
        })
        .collect();

    db.expect_get_jobs_by_types_and_statuses()
        .with(eq(vec![JobType::SnosRun]), eq(vec![JobStatus::PendingRetry, JobStatus::Created]), eq(None))
        .returning(move |_, _, _| Ok(pending_job_items.clone()));

    // Calculate the expected ranges for missing block queries
    let lower_limit = match latest_state_transition_block {
        None => min_block_to_process,
        Some(state_block) => std::cmp::max(state_block, min_block_to_process),
    };
    let upper_limit = std::cmp::min(sequencer_latest_block, max_block_to_process);

    // Mock get_missing_block_numbers_by_type_and_caps calls
    let missing_blocks_clone = missing_blocks.clone();
    let latest_snos_clone = latest_snos_completed_block;

    // Set up expectations for missing block queries
    if latest_snos_completed_block.is_none() {
        // Case: No SNOS completed - single call for entire range
        db.expect_get_missing_block_numbers_by_type_and_caps()
            .with(eq(JobType::SnosRun), eq(lower_limit as i64), eq(upper_limit as i64))
            .times(1)
            .returning(move |_, lower, upper| {
                let lower_u64 = lower as u64;
                let upper_u64 = upper as u64;
                let mut result: Vec<u64> = (lower_u64..=upper_u64).collect();
                result.retain(|&block| !expected_blocks.contains(&block) || missing_blocks_clone.contains(&block));
                Ok(result)
            });
    } else {
        let last_completed = latest_snos_completed_block.unwrap();

        // First call: missing blocks in [lower_limit, last_completed]
        if lower_limit <= last_completed && last_completed != 0 {
            let missing_in_first_range: Vec<u64> = missing_blocks_clone
                .iter()
                .filter(|&&block| block >= lower_limit && block <= last_completed)
                .cloned()
                .collect();

            db.expect_get_missing_block_numbers_by_type_and_caps()
                .with(eq(JobType::SnosRun), eq(lower_limit as i64), eq(last_completed as i64))
                .times(1)
                .returning(move |_, _, _| Ok(missing_in_first_range.clone()));
        }

        // Second call: new blocks in [last_completed+1, upper_limit]
        if last_completed < upper_limit {
            let start_block = last_completed + 1;
            db.expect_get_missing_block_numbers_by_type_and_caps()
                .with(eq(JobType::SnosRun), eq(start_block as i64), eq(upper_limit as i64))
                .times(1)
                .returning(move |_, lower, upper| {
                    let lower_u64 = lower as u64;
                    let upper_u64 = upper as u64;
                    let result: Vec<u64> = (lower_u64..=upper_u64).collect();
                    Ok(result)
                });
        }
    }

    // Mock job creation for expected jobs
    for &block_num in &expected_jobs_to_create {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();
        let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
        let job_item_clone = job_item.clone();

        job_handler
            .expect_create_job()
            .with(eq(block_num_str.clone()), mockall::predicate::always())
            .returning(move |_, _| Ok(job_item_clone.clone()));

        db.expect_create_job()
            .withf(move |item| item.internal_id == block_num_str)
            .returning(move |_| Ok(job_item.clone()));
    }

    // Mock queue operations
    queue
        .expect_send_message()
        .times(expected_jobs_to_create.len())
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

    // Set up job handler context
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Configure environment variables
    env::set_var("MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", max_block_to_process.to_string());
    env::set_var("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", min_block_to_process.to_string());
    env::set_var("MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_SNOS_JOBS", max_concurrent_jobs.to_string());

    // Create provider
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    // Build test configuration
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the SNOS trigger
    crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await?;

    println!(
        "Test completed successfully for scenario:\n\
         Sequencer: {}, Min: {}, Max: {}, Slots: {}\n\
         SNOS completed: {:?}, StateUpdate: {:?}\n\
         Missing: {:?}, Pending: {:?}\n\
         Expected jobs: {:?}",
        sequencer_latest_block,
        min_block_to_process,
        max_block_to_process,
        max_concurrent_jobs,
        latest_snos_completed_block,
        latest_state_transition_block,
        missing_blocks,
        pending_blocks,
        expected_jobs_to_create
    );

    Ok(())
}
