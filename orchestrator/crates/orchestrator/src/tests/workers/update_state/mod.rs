use std::sync::Arc;

use mockall::predicate::eq;
use rstest::*;
use uuid::Uuid;

use crate::constants::{BLOB_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::jobs::state_update_job::StateUpdateJob;
use crate::jobs::types::{JobStatus, JobType};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::utils::{create_and_store_prerequisite_jobs, get_job_item_mock_by_id};
use crate::workers::update_state::UpdateStateWorker;
use crate::workers::Worker;

#[rstest]
#[tokio::test]
async fn update_state_worker_with_pending_jobs() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let unique_id = Uuid::new_v4();
    let mut job_item = get_job_item_mock_by_id("1".to_string(), unique_id);
    job_item.status = JobStatus::PendingVerification;
    job_item.job_type = JobType::StateTransition;
    services.config.database().create_job_item(job_item).await.unwrap();

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    assert_eq!(latest_job.status, JobStatus::PendingVerification);
    assert_eq!(latest_job.job_type, JobType::StateTransition);
    assert_eq!(latest_job.id, unique_id);
}

#[rstest]
#[tokio::test]
async fn update_state_worker_first_block() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create both SNOS and DA jobs for block 0 with Completed status
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 0, JobStatus::Completed).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);

    // Get the blocks to settle from the StateUpdateMetadata
    let state_metadata: StateUpdateMetadata = latest_job.metadata.specific.clone().try_into().unwrap();
    assert_eq!(state_metadata.blocks_to_settle, vec![0]);
}

#[rstest]
#[tokio::test]
async fn update_state_worker_first_block_missing() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create both SNOS and DA jobs for block 2 with Completed status
    // Note: Block 0 and 1 are missing, so the worker should not create a job
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 2, JobStatus::Completed).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    // update state worker should not create any job
    assert!(services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn update_state_worker_selects_consective_blocks() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create both SNOS and DA jobs for blocks 0, 1, and 3 with Completed status
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 0, JobStatus::Completed).await.unwrap();
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 1, JobStatus::Completed).await.unwrap();
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 3, JobStatus::Completed).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    // update state worker should not create any job
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);

    // Get the blocks to settle from the StateUpdateMetadata
    let state_metadata: StateUpdateMetadata = latest_job.metadata.specific.clone().try_into().unwrap();
    assert_eq!(state_metadata.blocks_to_settle, vec![0, 1]);
}

#[rstest]
#[tokio::test]
async fn update_state_worker_continues_from_previous_state_update() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create both SNOS and DA jobs for block 5 with Completed status
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 5, JobStatus::Completed).await.unwrap();

    // add state transition job for blocks 0-4
    let mut job_item = get_job_item_mock_by_id("0".to_string(), Uuid::new_v4());
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::StateTransition;

    // Create proper StateUpdateMetadata with blocks 0-4
    let state_metadata = StateUpdateMetadata {
        blocks_to_settle: vec![0, 1, 2, 3, 4],
        snos_output_paths: vec![
            format!("{}/{}", 0, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 1, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 2, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 3, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 4, SNOS_OUTPUT_FILE_NAME),
        ],
        program_output_paths: vec![
            format!("{}/{}", 0, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 1, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 2, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 3, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 4, PROGRAM_OUTPUT_FILE_NAME),
        ],
        blob_data_paths: vec![
            format!("{}/{}", 0, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 1, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 2, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 3, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 4, BLOB_DATA_FILE_NAME),
        ],
        last_failed_block_no: None,
        tx_hashes: Vec::new(),
    };

    job_item.metadata =
        JobMetadata { common: CommonMetadata::default(), specific: JobSpecificMetadata::StateUpdate(state_metadata) };

    services.config.database().create_job_item(job_item).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    // update state worker should not create any job
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);

    // Get the blocks to settle from the StateUpdateMetadata
    let state_metadata: StateUpdateMetadata = latest_job.metadata.specific.clone().try_into().unwrap();
    assert_eq!(state_metadata.blocks_to_settle, vec![5]);
}

#[rstest]
#[tokio::test]
async fn update_state_worker_next_block_missing() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create both SNOS and DA jobs for block 6 with Completed status
    // Note: Block 5 is missing, so the worker should not create a job
    let (_, _) = create_and_store_prerequisite_jobs(services.config.clone(), 6, JobStatus::Completed).await.unwrap();

    // add state transition job for blocks 0-4
    let unique_id = Uuid::new_v4();
    let mut job_item = get_job_item_mock_by_id("0".to_string(), unique_id);
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::StateTransition;

    // Create proper StateUpdateMetadata with blocks 0-4
    let state_metadata = StateUpdateMetadata {
        blocks_to_settle: vec![0, 1, 2, 3, 4],
        snos_output_paths: vec![
            format!("{}/{}", 0, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 1, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 2, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 3, SNOS_OUTPUT_FILE_NAME),
            format!("{}/{}", 4, SNOS_OUTPUT_FILE_NAME),
        ],
        program_output_paths: vec![
            format!("{}/{}", 0, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 1, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 2, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 3, PROGRAM_OUTPUT_FILE_NAME),
            format!("{}/{}", 4, PROGRAM_OUTPUT_FILE_NAME),
        ],
        blob_data_paths: vec![
            format!("{}/{}", 0, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 1, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 2, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 3, BLOB_DATA_FILE_NAME),
            format!("{}/{}", 4, BLOB_DATA_FILE_NAME),
        ],
        last_failed_block_no: None,
        tx_hashes: Vec::new(),
    };

    job_item.metadata =
        JobMetadata { common: CommonMetadata::default(), specific: JobSpecificMetadata::StateUpdate(state_metadata) };

    services.config.database().create_job_item(job_item).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    assert_eq!(latest_job.id, unique_id);
}
