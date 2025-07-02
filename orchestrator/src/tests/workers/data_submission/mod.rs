#![allow(clippy::type_complexity)]
use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::constant::BLOB_DATA_FILE_NAME;
use crate::types::jobs::metadata::{DaMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::triggers::JobTrigger;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use rstest::rstest;
use std::error::Error;
use std::sync::Arc;
use uuid::Uuid;

#[rstest]
// Scenario 1: No completed proving jobs exist
// Expected result: no data submission jobs created
#[case(
    None,     // earliest_failed_block
    vec![],   // completed_proving_jobs (no completed proving jobs)
    vec![],   // expected_data_submission_jobs (no jobs to create)
    0         // expected_created_count
)]
// Scenario 2: Single completed proving job with valid metadata
// Expected result: one data submission job created
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("path/to/cairo_pie_0".to_string()), Some("valid_proof_hash_0".to_string()), Some(1000))
    ], // completed_proving_jobs (block_num, input_path, proof_hash, n_steps)
    vec![0],  // expected_data_submission_jobs
    1         // expected_created_count
)]
// Scenario 3: Multiple completed proving jobs with valid metadata
// Expected result: data submission jobs created for all
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("path/to/cairo_pie_0".to_string()), Some("proof_hash_0".to_string()), Some(1000)),
        (1, Some("path/to/cairo_pie_1".to_string()), Some("proof_hash_1".to_string()), Some(1500)),
        (2, Some("path/to/cairo_pie_2".to_string()), Some("proof_hash_2".to_string()), Some(2000))
    ], // completed_proving_jobs
    vec![0, 1, 2], // expected_data_submission_jobs
    3              // expected_created_count
)]
// Scenario 4: Proving jobs without input_path (should still create data submission job)
// Expected result: data submission jobs created (input_path is not required for data submission)
#[case(
    None,     // earliest_failed_block
    vec![
        (0, None, Some("proof_hash_0".to_string()), Some(1000)), // No input_path
        (1, Some("path/to/cairo_pie_1".to_string()), None, Some(1500)) // No proof hash
    ], // completed_proving_jobs
    vec![0, 1], // expected_data_submission_jobs (both should be created)
    2           // expected_created_count
)]
// Scenario 5: Proving jobs with earliest_failed_block constraint
// Expected result: data submission jobs created only for blocks before failed block
#[case(
    Some(1),  // earliest_failed_block (blocks >= 1 should be skipped)
    vec![
        (0, Some("path/to/cairo_pie_0".to_string()), Some("proof_hash_0".to_string()), Some(1000)), // Valid (< failed block)
        (1, Some("path/to/cairo_pie_1".to_string()), Some("proof_hash_1".to_string()), Some(1500)), // Skipped (>= failed block)
        (2, Some("path/to/cairo_pie_2".to_string()), Some("proof_hash_2".to_string()), Some(2000))  // Skipped (>= failed block)
    ], // completed_proving_jobs
    vec![0],  // expected_data_submission_jobs (only block 0)
    1         // expected_created_count
)]
// Scenario 6: All proving jobs are beyond failed block
// Expected result: no data submission jobs created (all skipped)
#[case(
    Some(0),  // earliest_failed_block (all blocks >= 0 should be skipped)
    vec![
        (0, Some("path/to/cairo_pie_0".to_string()), Some("proof_hash_0".to_string()), Some(1000)),
        (1, Some("path/to/cairo_pie_1".to_string()), Some("proof_hash_1".to_string()), Some(1500)),
        (2, Some("path/to/cairo_pie_2".to_string()), Some("proof_hash_2".to_string()), Some(2000))
    ], // completed_proving_jobs
    vec![],   // expected_data_submission_jobs (all skipped)
    0         // expected_created_count
)]
// Scenario 7: Large number of completed proving jobs
// Expected result: data submission jobs created for all valid ones
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("pie_0".to_string()), Some("hash_0".to_string()), Some(1000)),
        (1, Some("pie_1".to_string()), Some("hash_1".to_string()), Some(1100)),
        (2, Some("pie_2".to_string()), Some("hash_2".to_string()), Some(1200)),
        (3, Some("pie_3".to_string()), Some("hash_3".to_string()), Some(1300)),
        (4, Some("pie_4".to_string()), Some("hash_4".to_string()), Some(1400)),
        (5, Some("pie_5".to_string()), Some("hash_5".to_string()), Some(1500))
    ], // completed_proving_jobs
    vec![0, 1, 2, 3, 4, 5], // expected_data_submission_jobs
    6                       // expected_created_count
)]
// Scenario 8: Mix of valid proving jobs with failed block constraint
// Expected result: data submission jobs created only for valid blocks before failed block
#[case(
    Some(3),  // earliest_failed_block
    vec![
        (0, Some("pie_0".to_string()), Some("hash_0".to_string()), Some(1000)), // Valid
        (1, Some("pie_1".to_string()), Some("hash_1".to_string()), Some(1100)), // Valid
        (2, Some("pie_2".to_string()), Some("hash_2".to_string()), Some(1200)), // Valid
        (3, Some("pie_3".to_string()), Some("hash_3".to_string()), Some(1300)), // Skipped - at failed block
        (4, Some("pie_4".to_string()), Some("hash_4".to_string()), Some(1400)), // Skipped - beyond failed block
        (5, Some("pie_5".to_string()), Some("hash_5".to_string()), Some(1500))  // Skipped - beyond failed block
    ], // completed_proving_jobs
    vec![0, 1, 2], // expected_data_submission_jobs (only blocks before failed block)
    3              // expected_created_count
)]
// Scenario 9: Proving jobs with minimal metadata (testing edge cases)
// Expected result: data submission jobs created for all (minimal metadata is acceptable)
#[case(
    None,     // earliest_failed_block
    vec![
        (0, None, None, None), // Minimal metadata
        (1, None, None, Some(0)), // Zero n_steps
        (2, Some("".to_string()), Some("".to_string()), Some(1000)) // Empty strings
    ], // completed_proving_jobs
    vec![0, 1, 2], // expected_data_submission_jobs (all should be created)
    3              // expected_created_count
)]
// Scenario 10: High block numbers with failed block constraint
// Expected result: data submission jobs created only for blocks before failed block
#[case(
    Some(1000),  // earliest_failed_block
    vec![
        (999, Some("pie_999".to_string()), Some("hash_999".to_string()), Some(10000)),  // Valid
        (1000, Some("pie_1000".to_string()), Some("hash_1000".to_string()), Some(11000)), // Skipped - at failed block
        (1001, Some("pie_1001".to_string()), Some("hash_1001".to_string()), Some(12000))  // Skipped - beyond failed block
    ], // completed_proving_jobs
    vec![999], // expected_data_submission_jobs (only block 999)
    1          // expected_created_count
)]
#[tokio::test]
async fn test_data_submission_worker(
    #[case] earliest_failed_block: Option<u64>,
    #[case] completed_proving_jobs: Vec<(u64, Option<String>, Option<String>, Option<usize>)>, // (block_num, input_path, proof_hash, n_steps)
    #[case] expected_data_submission_jobs: Vec<u64>,
    #[case] expected_created_count: usize,
) -> Result<(), Box<dyn Error>> {
    dotenvy::from_filename_override(".env.test").expect("Failed to load the .env file");

    // Setup mock clients
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let mut job_handler = MockJobHandlerTrait::new();

    // Mock earliest_failed_block_number query
    db.expect_get_earliest_failed_block_number().returning(move || Ok(earliest_failed_block));

    // Create completed proving job items
    let proving_job_items: Vec<_> = completed_proving_jobs
        .iter()
        .map(|(block_num, input_path, proof_hash, n_steps)| {
            let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
            job_item.metadata.specific = JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: *block_num,
                input_path: input_path.as_ref().map(|path| ProvingInputType::CairoPie(path.clone())),
                download_proof: proof_hash.clone(),
                ensure_on_chain_registration: None, // Not needed for data submission
                n_steps: *n_steps,
            });
            job_item.status = JobStatus::Completed;
            job_item
        })
        .collect();

    // Mock database call to get proving jobs without data submission jobs
    let proving_jobs_clone = proving_job_items.clone();
    db.expect_get_jobs_without_successor()
        .with(eq(JobType::ProofCreation), eq(JobStatus::Completed), eq(JobType::DataSubmission))
        .returning(move |_, _, _| Ok(proving_jobs_clone.clone()));

    // Mock get_job_by_internal_id_and_type to always return None
    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    // Mock job creation for expected data submission jobs
    for &block_num in &expected_data_submission_jobs {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();

        // Only expect job creation for jobs that should actually be created
        // (i.e., are not beyond failed block)
        if earliest_failed_block.is_none() || block_num < earliest_failed_block.unwrap() {
            let mut da_job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
            da_job_item.metadata.specific = JobSpecificMetadata::Da(DaMetadata {
                block_number: block_num,
                blob_data_path: Some(format!("{}/{}", block_num, BLOB_DATA_FILE_NAME)),
                tx_hash: None, // Will be populated during processing
            });
            da_job_item.status = JobStatus::Created;

            let job_item_clone = da_job_item.clone();

            job_handler
                .expect_create_job()
                .with(eq(block_num_str.clone()), mockall::predicate::always())
                .returning(move |_, _| Ok(job_item_clone.clone()));

            let block_num_str_for_db = block_num_str.clone();
            db.expect_create_job()
                .withf(move |item| {
                    item.internal_id == block_num_str_for_db
                        && matches!(item.metadata.specific, JobSpecificMetadata::Da(_))
                })
                .returning(move |_| Ok(da_job_item.clone()));
        }
    }

    // Setup job handler context
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    // Mock queue operations for successful job creations
    queue
        .expect_send_message()
        .times(expected_created_count)
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _, _| *queue == QueueType::DataSubmissionJobProcessing);

    // Build test configuration
    let services = TestConfigBuilder::new()
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the Data Submission worker
    crate::worker::event_handler::triggers::data_submission::DataSubmissionJobTrigger
        .run_worker(services.config)
        .await?;

    println!(
        "✅ Test completed for scenario: earliest_failed_block={:?}, proving_jobs={}, expected_data_submission_jobs={:?}, created_count={}",
        earliest_failed_block, completed_proving_jobs.len(), expected_data_submission_jobs, expected_created_count
    );

    Ok(())
}
