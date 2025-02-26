use std::collections::HashMap;
use std::sync::Arc;

use mockall::predicate::eq;
use rstest::*;
use uuid::Uuid;

use crate::jobs::constants::JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::state_update_job::StateUpdateJob;
use crate::jobs::types::{JobStatus, JobType};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::utils::get_job_item_mock_by_id;
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
    services.config.database().create_job(job_item).await.unwrap();

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

    let unique_id = Uuid::new_v4();
    let mut job_item = get_job_item_mock_by_id("0".to_string(), unique_id);
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);
    assert_eq!(latest_job.metadata.get(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY).unwrap(), "0");
}

#[rstest]
#[tokio::test]
async fn update_state_worker_first_block_missing() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // skip first block from DA completion
    let mut job_item = get_job_item_mock_by_id("2".to_string(), Uuid::new_v4());
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item).await.unwrap();

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

    let mut job_item_one = get_job_item_mock_by_id("0".to_string(), Uuid::new_v4());
    job_item_one.status = JobStatus::Completed;
    job_item_one.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item_one).await.unwrap();

    let mut job_item_two = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());
    job_item_two.status = JobStatus::Completed;
    job_item_two.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item_two).await.unwrap();

    // skip block 3
    let mut job_item_three = get_job_item_mock_by_id("3".to_string(), Uuid::new_v4());
    job_item_three.status = JobStatus::Completed;
    job_item_three.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item_three).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    // update state worker should not create any job
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);
    assert_eq!(latest_job.metadata.get(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY).unwrap(), "0,1");
}

#[rstest]
#[tokio::test]
async fn update_state_worker_continues_from_previous_state_update() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // add DA completion job for block 5
    let mut job_item = get_job_item_mock_by_id("5".to_string(), Uuid::new_v4());
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item).await.unwrap();

    // add state transition job for blocks 0-4
    let mut job_item = get_job_item_mock_by_id("0".to_string(), Uuid::new_v4());
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::StateTransition;
    let mut metadata = HashMap::new();
    metadata.insert(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(), "0,1,2,3,4".to_string());
    job_item.metadata = metadata;
    services.config.database().create_job(job_item).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    println!("latest job item {:?}", latest_job);
    // update state worker should not create any job
    assert_eq!(latest_job.status, JobStatus::Created);
    assert_eq!(latest_job.job_type, JobType::StateTransition);
    assert_eq!(latest_job.metadata.get(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY).unwrap(), "5");
}

#[rstest]
#[tokio::test]
async fn update_state_worker_next_block_missing() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // add DA completion job for block 5
    let mut job_item = get_job_item_mock_by_id("6".to_string(), Uuid::new_v4());
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::DataSubmission;
    services.config.database().create_job(job_item).await.unwrap();

    // add state transition job for blocks 0-4
    let unique_id = Uuid::new_v4();
    let mut job_item = get_job_item_mock_by_id("0".to_string(), unique_id);
    job_item.status = JobStatus::Completed;
    job_item.job_type = JobType::StateTransition;
    let mut metadata = HashMap::new();
    metadata.insert(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY.to_string(), "0,1,2,3,4".to_string());
    job_item.metadata = metadata;
    services.config.database().create_job(job_item).await.unwrap();

    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().with(eq(JobType::StateTransition)).returning(move |_| Arc::new(Box::new(StateUpdateJob)));

    let update_state_worker = UpdateStateWorker {};
    assert!(update_state_worker.run_worker(services.config.clone()).await.is_ok());

    let latest_job =
        services.config.database().get_latest_job_by_type(JobType::StateTransition).await.unwrap().unwrap();
    assert_eq!(latest_job.id, unique_id);
}
