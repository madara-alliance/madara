// use crate::core::client::database::MockDatabaseClient;
// use crate::core::client::queue::MockQueueClient;
// use crate::tests::config::TestConfigBuilder;
// use crate::tests::workers::utils::get_job_item_mock_by_id;
// use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SnosMetadata, StateUpdateMetadata};
// use crate::types::jobs::types::{JobStatus, JobType};
// use crate::types::queue::QueueType;
// use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
// use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
// use crate::worker::event_handler::triggers::JobTrigger;
// use httpmock::MockServer;
// use mockall::predicate::eq;
// use orchestrator_da_client_interface::MockDaClient;
// use rstest::rstest;
// use serde_json::json;
// use std::env;
// use starknet::providers::jsonrpc::HttpTransport;
// use starknet::providers::JsonRpcClient;
// use std::error::Error;
// use std::sync::Arc;
// use url::Url;
// use uuid::Uuid;

// #[rstest]
// // Scenario 1: No previous jobs, should create jobs from min_block (0) up to max_concurrent (3)
// // State: No completed jobs | Max_concurrent: 3 | Min: 0 | Max: 100000 | Latest: 100000
// // Expected: Create jobs for blocks [0, 1, 2]
// #[case(None, None, vec![], vec![], vec![], vec![0, 1, 2])]

// // Scenario 2: Has completed SNOS job at block 5, no missing blocks, should create new jobs after block 5
// // State: Latest SNOS: 5 | No missing blocks | Max_concurrent: 3
// // Expected: Create jobs for blocks [6, 7, 8]
// #[case(Some(5), None, vec![], vec![], vec![], vec![6, 7, 8])]

// // Scenario 3: Has completed SNOS job at block 5, has missing blocks 2,3, should fill missing first then new
// // State: Latest SNOS: 5 | Missing: [2, 3] | Max_concurrent: 3
// // Expected: Create jobs for blocks [2, 3, 6] (2 missing + 1 new)
// #[case(Some(5), None, vec![0, 1, 2, 3, 4], vec![], vec![], vec![0, 1, 2])]

// // Scenario 4: Has completed StateTransition for blocks [0,1,2], SNOS completed till block 1
// // State: Latest SNOS: 1 | Latest StateTransition covers blocks [0,1,2] | Max_concurrent: 3
// // Expected: Create jobs for blocks [2] (from StateTransition bound) then [2, 3, 4] but 2 is duplicate, so [2, 3, 4]
// #[case(Some(1), Some(vec![0, 1, 2]), vec![], vec![], vec![], vec![2, 3, 4])]

// // Scenario 5: Has pending/created jobs that consume some slots
// // State: Latest SNOS: 3 | Pending jobs: [4] | Created jobs: [5] | Max_concurrent: 3
// // Expected: Only 1 slot available, create job for block [6]
// #[case(Some(3), None, vec![], vec![4], vec![5], vec![6])]

// // Scenario 6: All slots occupied by pending jobs
// // State: Latest SNOS: 2 | Pending jobs: [3, 4, 5] | Max_concurrent: 3
// // Expected: No jobs created (all slots occupied)
// #[case(Some(2), None, vec![], vec![3, 4, 5], vec![], vec![])]

// // // Scenario 7: Complex scenario with missing blocks and limited slots
// // // State: Latest SNOS: 8 | Missing: [3, 5, 7] | Pending: [6] | Max_concurrent: 4
// // // Expected: Fill missing [3, 5, 7] (3 slots) + create [9] (1 remaining slot) = [3, 5, 7, 9]
// #[case(Some(8), None, vec![3, 5, 7], vec![6], vec![], vec![3, 5, 7, 9])]

// // // Scenario 8: StateTransition constraint limits lower bound
// // // State: Latest SNOS: 10 | Latest StateTransition covers [5,6,7,8] | Missing from DB: [2, 3, 9] | Max_concurrent: 3
// // // Expected: Lower bound is max(0, 8) = 8, so missing blocks [2,3] are filtered out, only [9] remains + new blocks [11, 12]
// #[case(Some(10), Some(vec![5, 6, 7, 8]), vec![2, 3, 9], vec![], vec![], vec![9, 11, 12])]

// // Scenario 9: Upper limit constraint (latest_created_block_from_sequencer = 5 instead of 100000)
// // This is handled by the mock but represents the case where sequencer has limited blocks
// // State: Latest SNOS: 3 | Latest from sequencer: 5 | Max_concurrent: 5
// // Expected: Create jobs [4, 5] (limited by sequencer)
// // Note: This would need additional test setup to mock the sequencer response differently

// #[tokio::test]
// async fn test_snos_worker(
//     #[case] latest_snos_block: Option<u64>,
//     #[case] latest_state_transition_blocks: Option<Vec<u64>>,
//     #[case] missing_blocks_from_db: Vec<u64>,
//     #[case] pending_retry_blocks: Vec<u64>,
//     #[case] created_blocks: Vec<u64>,
//     #[case] expected_jobs_to_create: Vec<u64>,
// ) -> Result<(), Box<dyn Error>> {
//     let server = MockServer::start();
//     let da_client = MockDaClient::new();
//     let mut db = MockDatabaseClient::new();
//     let mut queue = MockQueueClient::new();
//     let mut job_handler = MockJobHandlerTrait::new();

//     // Mock get_latest_job_by_type_and_status for SNOS jobs
//     let latest_snos_job = latest_snos_block.map(|block_num| {
//         let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
//         job_item.metadata.specific = JobSpecificMetadata::Snos(SnosMetadata {
//             block_number: block_num,
//             ..Default::default()
//         });
//         job_item
//     });

//     db.expect_get_latest_job_by_type_and_status()
//         .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
//         .returning(move |_, _| Ok(latest_snos_job.clone()));

//     // Mock get_latest_job_by_type_and_status for StateTransition jobs
//     let latest_state_transition_job = latest_state_transition_blocks.map(|blocks| {
//         let mut job_item = get_job_item_mock_by_id("state_transition".to_string(), Uuid::new_v4());
//         job_item.metadata.specific = JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
//             blocks_to_settle: blocks,
//             ..Default::default()
//         });
//         job_item
//     });

//     db.expect_get_latest_job_by_type_and_status()
//         .with(eq(JobType::StateTransition), eq(JobStatus::Completed))
//         .returning(move |_, _| Ok(latest_state_transition_job.clone()));

//     // Mock get_jobs_by_types_and_statuses for pending/created jobs
//     let pending_and_created_job_items: Vec<_> = pending_retry_blocks
//         .iter()
//         .chain(created_blocks.iter())
//         .map(|block_num| {
//             get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4())
//         })
//         .collect();

//     db.expect_get_jobs_by_types_and_statuses()
//         .with(
//             eq(vec![JobType::SnosRun]),
//             eq(vec![JobStatus::PendingRetry, JobStatus::Created]),
//             eq(None)
//         )
//         .returning(move |_, _, _| Ok(pending_and_created_job_items.clone()));

//     // Mock get_missing_block_numbers_by_type_and_caps
//     // This needs to return the missing blocks within the specified range
//     let missing_blocks_clone = missing_blocks_from_db.clone();
//     db.expect_get_missing_block_numbers_by_type_and_caps()
//         .withf(move |job_type, _lower, _upper| *job_type == JobType::SnosRun)
//         .returning(move |_, _, _| Ok(missing_blocks_clone.clone()));

//     // Mock get_job_by_internal_id_and_type to always return None
//     db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

//     // Setup job creation expectations for each expected job
//     for &block_num in &expected_jobs_to_create {
//         let uuid = Uuid::new_v4();
//         let block_num_str = block_num.to_string();
//         let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
//         let job_item_clone = job_item.clone();

//         job_handler
//             .expect_create_job()
//             .with(eq(block_num_str.clone()), mockall::predicate::always())
//             .returning(move |_, _| Ok(job_item_clone.clone()));

//         let block_num_str_for_db = block_num_str.clone();
//         db.expect_create_job()
//             .withf(move |item| item.internal_id == block_num_str_for_db)
//             .returning(move |_| Ok(job_item.clone()));
//     }

//     let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
//     let ctx = get_job_handler_context();
//     ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

//     // Queue function call simulations
//     queue
//         .expect_send_message()
//         .times(expected_jobs_to_create.len())
//         .returning(|_, _, _| Ok(()))
//         .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

//     // Mock RPC response for block_number (latest_created_block_from_sequencer = 100000)
//     let response = json!({ "id": 1, "jsonrpc": "2.0", "result": 100000 });

//     let provider = JsonRpcClient::new(HttpTransport::new(
//         Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
//     ));

//     // Configure test environment with custom service config
//     let mut services_builder = TestConfigBuilder::new()
//         .configure_starknet_client(provider.into())
//         .configure_database(db.into())
//         .configure_queue_client(queue.into())
//         .configure_da_client(da_client.into());

//     env::set_var("MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", "100000");
//     env::set_var("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", "0");
//     env::set_var("MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_SNOS_JOBS", "3");

//     let services = services_builder.build().await;

//     // Mock block_number RPC call - note this is currently hardcoded to 100000 in the implementation
//     let rpc_block_call_mock = server.mock(|when, then| {
//         when.path("/").body_includes("starknet_blockNumber");
//         then.status(200).body(serde_json::to_vec(&response).unwrap());
//     });

//     // Run the worker
//     crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await?;

//     // Verify RPC call was made (though currently commented out in implementation)
//     // rpc_block_call_mock.assert();

//     Ok(())
// }

// // // Additional test for edge case with custom limits
// // #[tokio::test]
// // async fn test_snos_worker_with_custom_limits() -> Result<(), Box<dyn Error>> {
// //     let server = MockServer::start();
// //     let da_client = MockDaClient::new();
// //     let mut db = MockDatabaseClient::new();
// //     let mut queue = MockQueueClient::new();
// //     let mut job_handler = MockJobHandlerTrait::new();

// //     // Test scenario: min_block = 10, max_block = 20, max_concurrent = 2
// //     // Latest SNOS: 12, should create jobs [13, 14] (limited by max_concurrent)

// //     // No latest SNOS job
// //     db.expect_get_latest_job_by_type_and_status()
// //         .with(eq(JobType::SnosRun), eq(JobStatus::Completed))
// //         .returning(|_, _| Ok(Some({
// //             let mut job_item = get_job_item_mock_by_id("12".to_string(), Uuid::new_v4());
// //             job_item.metadata.specific = JobSpecificMetadata::Snos(SnosMetadata {
// //                 block_number: 12,
// //                 ..Default::default()
// //             });
// //             job_item
// //         })));

// //     // No latest StateTransition job
// //     db.expect_get_latest_job_by_type_and_status()
// //         .with(eq(JobType::StateTransition), eq(JobStatus::Completed))
// //         .returning(|_, _| Ok(None));

// //     // No pending jobs
// //     db.expect_get_jobs_by_types_and_statuses()
// //         .with(
// //             eq(vec![JobType::SnosRun]),
// //             eq(vec![JobStatus::PendingRetry, JobStatus::Created]),
// //             eq(None)
// //         )
// //         .returning(|_, _, _| Ok(vec![]));

// //     // No missing blocks
// //     db.expect_get_missing_block_numbers_by_type_and_caps()
// //         .returning(|_, _, _| Ok(vec![]));

// //     db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

// //     // Expect jobs for blocks 13, 14
// //     for &block_num in &[13u64, 14u64] {
// //         let uuid = Uuid::new_v4();
// //         let block_num_str = block_num.to_string();
// //         let job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
// //         let job_item_clone = job_item.clone();

// //         job_handler
// //             .expect_create_job()
// //             .with(eq(block_num_str.clone()), mockall::predicate::always())
// //             .returning(move |_, _| Ok(job_item_clone.clone()));

// //         let block_num_str_for_db = block_num_str.clone();
// //         db.expect_create_job()
// //             .withf(move |item| item.internal_id == block_num_str_for_db)
// //             .returning(move |_| Ok(job_item.clone()));
// //     }

// //     let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
// //     let ctx = get_job_handler_context();
// //     ctx.expect().with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

// //     queue
// //         .expect_send_message()
// //         .times(2)
// //         .returning(|_, _, _| Ok(()))
// //         .withf(|queue, _payload, _delay| *queue == QueueType::SnosJobProcessing);

// //     let provider = JsonRpcClient::new(HttpTransport::new(
// //         Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
// //     ));

// //     env::set_var("MADARA_ORCHESTRATOR_MAX_BLOCK_NO_TO_PROCESS", "20");
// //     env::set_var("MADARA_ORCHESTRATOR_MIN_BLOCK_NO_TO_PROCESS", "10");
// //     env::set_var("MADARA_ORCHESTRATOR_MAX_CONCURRENT_CREATED_SNOS_JOBS", "2");

// //     let services = TestConfigBuilder::new()
// //         .configure_starknet_client(provider.into())
// //         .configure_database(db.into())
// //         .configure_queue_client(queue.into())
// //         .configure_da_client(da_client.into())
// //         .build()
// //         .await;

// //     crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await?;

// //     Ok(())
// // }
