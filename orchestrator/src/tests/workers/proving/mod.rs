#![allow(clippy::type_complexity)]
use std::error::Error;
use std::sync::Arc;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::metadata::{ProvingInputType, ProvingMetadata, SnosMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::triggers::JobTrigger;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use rstest::rstest;
use uuid::Uuid;

#[rstest]
// Scenario 1: No completed SNOS jobs exist
// Expected result: no proving jobs created
#[case(
    None,     // earliest_failed_block
    vec![],   // completed_snos_jobs (no completed SNOS jobs)
    vec![],   // expected_proving_jobs (no jobs to create)
    0         // expected_created_count
)]
// Scenario 2: Single completed SNOS job with valid snos_fact
// Expected result: one proving job created
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), Some("path/to/cairo_pie_0".to_string()), Some(1000))
    ], // completed_snos_jobs
    vec![0],  // expected_proving_jobs
    1         // expected_created_count
)]
// Scenario 3: Multiple completed SNOS jobs with valid snos_facts
// Expected result: proving jobs created for all
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), Some("path/to/cairo_pie_0".to_string()), Some(1000)),
        (1, Some("valid_snos_fact_block_1".to_string()), Some("path/to/cairo_pie_1".to_string()), Some(1500)),
        (2, Some("valid_snos_fact_block_2".to_string()), Some("path/to/cairo_pie_2".to_string()), Some(2000))
    ], // completed_snos_jobs
    vec![0, 1, 2], // expected_proving_jobs
    3              // expected_created_count
)]
// Scenario 4: SNOS job without snos_fact (should be skipped)
// Expected result: no proving jobs created
#[case(
    None,     // earliest_failed_block
    vec![
        (0, None, Some("path/to/cairo_pie_0".to_string()), Some(1000)) // Missing snos_fact
    ], // completed_snos_jobs
    vec![],   // expected_proving_jobs (skipped due to missing fact)
    0         // expected_created_count
)]
// Scenario 5: Mix of valid and invalid SNOS jobs
// Expected result: proving jobs created only for valid ones
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), Some("path/to/cairo_pie_0".to_string()), Some(1000)), // Valid
        (1, None, Some("path/to/cairo_pie_1".to_string()), Some(1500)), // Invalid - no snos_fact
        (2, Some("valid_snos_fact_block_2".to_string()), Some("path/to/cairo_pie_2".to_string()), Some(2000))  // Valid
    ], // completed_snos_jobs
    vec![0, 2], // expected_proving_jobs (skip block 1)
    2           // expected_created_count
)]
// Scenario 6: Completed SNOS jobs but earliest_failed_block constraint blocks some
// Expected result: proving jobs created only for blocks before failed block
#[case(
    Some(1),  // earliest_failed_block (blocks >= 1 should be skipped)
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), Some("path/to/cairo_pie_0".to_string()), Some(1000)), // Valid (< failed block)
        (1, Some("valid_snos_fact_block_1".to_string()), Some("path/to/cairo_pie_1".to_string()), Some(1500)), // Skipped (>= failed block)
        (2, Some("valid_snos_fact_block_2".to_string()), Some("path/to/cairo_pie_2".to_string()), Some(2000))  // Skipped (>= failed block)
    ], // completed_snos_jobs
    vec![0],  // expected_proving_jobs (only block 0)
    1         // expected_created_count
)]
// Scenario 7: All SNOS jobs are beyond failed block
// Expected result: no proving jobs created (all skipped)
#[case(
    Some(0),  // earliest_failed_block (all blocks >= 0 should be skipped)
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), Some("path/to/cairo_pie_0".to_string()), Some(1000)),
        (1, Some("valid_snos_fact_block_1".to_string()), Some("path/to/cairo_pie_1".to_string()), Some(1500)),
        (2, Some("valid_snos_fact_block_2".to_string()), Some("path/to/cairo_pie_2".to_string()), Some(2000))
    ], // completed_snos_jobs
    vec![],   // expected_proving_jobs (all skipped)
    0         // expected_created_count
)]
// Scenario 8: Large number of completed SNOS jobs
// Expected result: proving jobs created for all valid ones
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("fact_0".to_string()), Some("pie_0".to_string()), Some(1000)),
        (1, Some("fact_1".to_string()), Some("pie_1".to_string()), Some(1100)),
        (2, Some("fact_2".to_string()), Some("pie_2".to_string()), Some(1200)),
        (3, Some("fact_3".to_string()), Some("pie_3".to_string()), Some(1300)),
        (4, Some("fact_4".to_string()), Some("pie_4".to_string()), Some(1400))
    ], // completed_snos_jobs
    vec![0, 1, 2, 3, 4], // expected_proving_jobs
    5                    // expected_created_count
)]
// Scenario 9: SNOS jobs without cairo_pie_path (should still create proving job)
// Expected result: proving job created with None input_path
#[case(
    None,     // earliest_failed_block
    vec![
        (0, Some("valid_snos_fact_block_0".to_string()), None, Some(1000)) // No cairo_pie_path
    ], // completed_snos_jobs
    vec![0],  // expected_proving_jobs (should still be created)
    1         // expected_created_count
)]
// Scenario 10: Complex scenario with failed block constraint and mixed validity
// Expected result: proving jobs created only for valid blocks before failed block
#[case(
    Some(3),  // earliest_failed_block
    vec![
        (0, Some("fact_0".to_string()), Some("pie_0".to_string()), Some(1000)), // Valid
        (1, None, Some("pie_1".to_string()), Some(1100)),                       // Invalid - no fact
        (2, Some("fact_2".to_string()), Some("pie_2".to_string()), Some(1200)), // Valid
        (3, Some("fact_3".to_string()), Some("pie_3".to_string()), Some(1300)), // Skipped - at failed block
        (4, Some("fact_4".to_string()), Some("pie_4".to_string()), Some(1400))  // Skipped - beyond failed block
    ], // completed_snos_jobs
    vec![0, 2], // expected_proving_jobs (only valid blocks before failed block)
    2           // expected_created_count
)]
#[tokio::test]
async fn test_proving_worker(
    #[case] earliest_failed_block: Option<u64>,
    #[case] completed_snos_jobs: Vec<(u64, Option<String>, Option<String>, Option<usize>)>, // (block_num, snos_fact, cairo_pie_path, n_steps)
    #[case] expected_proving_jobs: Vec<u64>,
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

    // Create completed SNOS job items
    let snos_job_items: Vec<_> = completed_snos_jobs
        .iter()
        .map(|(block_num, snos_fact, cairo_pie_path, n_steps)| {
            let mut job_item = get_job_item_mock_by_id(block_num.to_string(), Uuid::new_v4());
            job_item.metadata.specific = JobSpecificMetadata::Snos(SnosMetadata {
                block_number: *block_num,
                snos_fact: snos_fact.clone(),
                cairo_pie_path: cairo_pie_path.clone(),
                snos_n_steps: *n_steps,
                ..Default::default()
            });
            job_item.status = JobStatus::Completed;
            job_item
        })
        .collect();

    // Mock database call to get SNOS jobs without proving jobs
    let snos_jobs_clone = snos_job_items.clone();
    db.expect_get_jobs_without_successor()
        .with(eq(JobType::SnosRun), eq(JobStatus::Completed), eq(JobType::ProofCreation))
        .returning(move |_, _, _| Ok(snos_jobs_clone.clone()));

    // Mock get_job_by_internal_id_and_type to always return None
    db.expect_get_job_by_internal_id_and_type().returning(|_, _| Ok(None));

    // Mock job creation for expected proving jobs
    for &block_num in &expected_proving_jobs {
        let uuid = Uuid::new_v4();
        let block_num_str = block_num.to_string();

        // Find the corresponding SNOS job to get its metadata
        let snos_job = completed_snos_jobs.iter().find(|(b, _, _, _)| *b == block_num).unwrap();
        let (_, snos_fact, cairo_pie_path, n_steps) = snos_job;

        // Only expect job creation for jobs that should actually be created
        // (i.e., have snos_fact and are not beyond failed block)
        if snos_fact.is_some() && (earliest_failed_block.is_none() || block_num < earliest_failed_block.unwrap()) {
            let mut proving_job_item = get_job_item_mock_by_id(block_num_str.clone(), uuid);
            proving_job_item.metadata.specific = JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: block_num,
                input_path: cairo_pie_path.as_ref().map(|path| ProvingInputType::CairoPie(path.clone())),
                download_proof: None,
                ensure_on_chain_registration: snos_fact.clone(),
                n_steps: *n_steps,
            });
            proving_job_item.status = JobStatus::Created;

            let job_item_clone = proving_job_item.clone();

            job_handler
                .expect_create_job()
                .with(eq(block_num_str.clone()), mockall::predicate::always())
                .returning(move |_, _| Ok(job_item_clone.clone()));

            let block_num_str_for_db = block_num_str.clone();
            db.expect_create_job()
                .withf(move |item| {
                    item.internal_id == block_num_str_for_db
                        && matches!(item.metadata.specific, JobSpecificMetadata::Proving(_))
                })
                .returning(move |_| Ok(proving_job_item.clone()));
        }
    }

    // Setup job handler context
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context();
    ctx.expect().with(eq(JobType::ProofCreation)).returning(move |_| Arc::clone(&job_handler));

    // Mock queue operations for successful job creations
    queue
        .expect_send_message()
        .times(expected_created_count)
        .returning(|_, _, _| Ok(()))
        .withf(|queue, _, _| *queue == QueueType::ProvingJobProcessing);

    // Build test configuration
    let services = TestConfigBuilder::new()
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the Proving worker
    crate::worker::event_handler::triggers::proving::ProvingJobTrigger.run_worker(services.config).await?;

    Ok(())
}
