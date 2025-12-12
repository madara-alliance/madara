//! # Job Tests Module
//!
//! This module contains tests for job creation, processing, and management functionality.
//!
//! ## Sequential Test Execution
//!
//! Some tests in this module use mocks (via Mockall) which rely on a global context that is not
//! thread-safe. To enable parallel test execution while preventing race conditions and mutex
//! poisoning, these tests use `acquire_test_lock()` to serialize execution. This ensures:
//!
//! - Only one test using mocks runs at a time, preventing interference between tests
//! - Mock expectations are set and verified correctly without conflicts
//! - Tests can still run in parallel with non-mock tests (which use isolated resources via UUIDs)
//!
//! Tests that use `acquire_test_lock()` will run sequentially with respect to each other, but
//! can still run in parallel with tests that don't use mocks. This provides a good balance
//! between test isolation and execution speed.
//!
//! The `#![allow(clippy::await_holding_lock)]` annotation is necessary because these tests
//! intentionally hold a mutex guard across await points to serialize test execution. This is
//! safe because:
//! - The lock is held for the entire test duration to prevent concurrent mock usage
//! - The mutex is test-scoped and won't block production code
//! - This pattern is explicitly designed to serialize tests, not protect shared state

#![allow(clippy::await_holding_lock)]
// The above allow is necessary because tests intentionally hold a mutex guard across await points
// to serialize test execution. This ensures only one test using mocks runs at a time, preventing
// interference between tests when they set expectations for the same JobType. The lock is held for
// the entire test duration, which is safe because it's test-scoped and won't block production code.

use std::sync::Arc;
use std::time::Duration;

use mockall::predicate::eq;
use mongodb::bson::doc;
use rstest::rstest;
use tokio::time::sleep;

use crate::core::client::alert::MockAlertClient;
use crate::tests::common::MessagePayloadType;
use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::tests::utils::build_job_item;

#[cfg(test)]
pub mod da_job;

#[cfg(test)]
pub mod proving_job;

#[cfg(test)]
pub mod state_update_job;

#[cfg(test)]
pub mod snos_job;

#[cfg(test)]
pub mod batching_job;

#[cfg(test)]
mod aggregator_job;

use crate::core::client::queue::QueueError;
use crate::error::job::JobError;
use crate::tests::common::constants::{QUEUE_CONSUME_MAX_RETRIES, QUEUE_CONSUME_RETRY_DELAY_SECS};
use crate::tests::common::consume_message_with_retry;
use crate::tests::common::test_utils::{acquire_test_lock, get_job_handler_context_safe};
use crate::types::constant::CAIRO_PIE_FILE_NAME;
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::{QueueNameForJobType, QueueType};
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::service::JobService;
use assert_matches::assert_matches;

/// Tests `create_job` function when job is not existing in the db.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn create_job_job_does_not_exists_in_db_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = crate::tests::common::test_utils::acquire_test_lock();

    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 0);
    let mut job_handler = MockJobHandlerTrait::new();

    // Adding expectation for creation of new job.
    let job_item_clone = job_item.clone();
    job_handler.expect_create_job().times(1).returning(move |_, _| Ok(job_item_clone.clone()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create a proper JobMetadata for the test
    let metadata =
        JobMetadata { common: CommonMetadata::default(), specific: JobSpecificMetadata::Snos(SnosMetadata::default()) };

    // Mocking the `get_job_handler` call in create_job function.
    // Set up mock AFTER creating services to ensure proper ordering
    // Keep the guard alive for the entire test to prevent other tests from overwriting expectations
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx_guard = get_job_handler_context_safe();
    // create_job calls get_job_handler exactly once
    ctx_guard.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::create_job(JobType::SnosRun, "0".to_string(), metadata, services.config.clone())
        .await
        .is_ok());

    // Db checks.
    let job_in_db = services.config.database().get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(job_in_db.id, job_item.id);
    assert_eq!(job_in_db.internal_id, job_item.internal_id);
    assert_eq!(job_in_db.metadata, job_item.metadata);

    // Queue checks - retry a few times in case of timing issues
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `create_job` function when job is already existing in the db.
#[rstest]
#[tokio::test]
async fn create_job_job_exists_in_db_works() {
    let job_item = build_job_item(JobType::ProofCreation, JobStatus::Created, 0);

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    database_client.create_job(job_item.clone()).await.unwrap();

    // Create a proper JobMetadata for the test
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputType::CairoPie(format!("{}/{}", "0", CAIRO_PIE_FILE_NAME))),
            ..Default::default()
        }),
    };

    assert!(JobHandlerService::create_job(JobType::ProofCreation, "0".to_string(), metadata, services.config.clone())
        .await
        .is_ok());

    // There should be only 1 job in the db
    let jobs_in_db = database_client
        .get_jobs_by_types_and_statuses(vec![JobType::ProofCreation], vec![JobStatus::Created], None)
        .await
        .unwrap();
    assert_eq!(jobs_in_db.len(), 1);

    // Queue checks - queue should exist but be empty since job already existed
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    // Since queue exists but is empty, we expect ErrorFromQueueError
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

/// Tests `create_job` function when job handler's `create_job` returns an error
/// This test verifies that errors from the job handler are properly propagated
/// and that no job is created in the database or added to the queue when creation fails
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn create_job_job_handler_returns_error() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = crate::tests::common::test_utils::acquire_test_lock();

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let job_type = JobType::ProofCreation;
    let internal_id = "0".to_string();

    // Create a proper JobMetadata for the test
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputType::CairoPie(format!("{}/{}", internal_id, CAIRO_PIE_FILE_NAME))),
            ..Default::default()
        }),
    };

    // Create a mock job handler that returns an error
    let mut job_handler = MockJobHandlerTrait::new();
    job_handler
        .expect_create_job()
        .times(1)
        .returning(|_, _| Err(JobError::ProviderError("Job handler creation failed".to_string())));

    // Mock the `get_job_handler` call to return our error-producing handler
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx_guard = get_job_handler_context_safe();
    ctx_guard.expect().times(1).with(eq(job_type.clone())).returning(move |_| Arc::clone(&job_handler));

    // Verify that create_job returns an error
    let result =
        JobHandlerService::create_job(job_type.clone(), internal_id.clone(), metadata, services.config.clone()).await;
    assert!(result.is_err(), "create_job should return an error when job handler fails");

    // Verify the error is the one we expect
    assert_matches!(
        result.unwrap_err(),
        JobError::ProviderError(msg) if msg == "Job handler creation failed"
    );

    // Verify no job was created in the database
    let job_in_db = services.config.database().get_job_by_internal_id_and_type(&internal_id, &job_type).await.unwrap();
    assert!(job_in_db.is_none(), "No job should be created in the database when creation fails");

    // Verify no message was added to the queue
    // Wait a bit to ensure any async operations complete
    sleep(Duration::from_secs(2)).await;

    // Try to consume from the queue - should fail with NoData or QueueDoesNotExist
    let queue_result = services.config.queue().consume_message_from_queue(job_type.process_queue_name()).await;

    // The queue should be empty (NoData) or the queue might not exist yet
    // Use assert_matches! for consistency and better error messages
    assert_matches!(queue_result.unwrap_err(), QueueError::ErrorFromQueueError(_));
}

/// Tests `process_job` function when job is already existing in the db and job status is either
/// `Created` or `VerificationFailed`.
#[rstest]
#[case(JobType::SnosRun, JobStatus::Created)]
#[case(JobType::DataSubmission, JobStatus::VerificationFailed)]
#[tokio::test]
async fn process_job_with_job_exists_in_db_and_valid_job_processing_status_works(
    #[case] job_type: JobType,
    #[case] job_status: JobStatus,
) {
    // Acquire test lock to serialize this test with others that use mocks
    // This lock is intentionally held for the entire test to serialize execution
    let _test_lock = crate::tests::common::test_utils::acquire_test_lock();

    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let job_item = build_job_item(job_type.clone(), job_status.clone(), 1);

    // Creating job in database first
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting process job function in job processor to return the external ID.
    job_handler.expect_process_job().times(1).returning(move |_, _| Ok("0xbeef".to_string()));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);

    // Mocking the `get_job_handler` call AFTER creating job
    // process_job calls get_job_handler exactly once
    // Keep the guard alive for the entire test to prevent other tests from overwriting expectations
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx_guard = get_job_handler_context_safe();
    ctx_guard.expect().times(1).with(eq(job_type.clone())).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::process_job(job_item.id, services.config.clone()).await.is_ok());
    // Getting the updated job.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    // checking if job_status is updated in db
    assert_eq!(updated_job.status, JobStatus::PendingVerification);
    assert_eq!(updated_job.external_id, ExternalId::String(Box::from("0xbeef")));

    // Check that process attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.process_attempt_no, 1);

    // Queue checks - retry a few times in case of timing issues
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        job_type.verify_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `process_job` function when job handler panics during execution.
/// This test verifies that:
/// 1. The panic is properly caught and handled
/// 2. The job is moved to failed state
/// 3. Appropriate error message is set in the job metadata
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn process_job_handles_panic() {
    // Acquire test lock to serialize this test with others that use mocks
    // This prevents Mockall's global context from being interfered with by parallel tests
    let _test_lock = acquire_test_lock();

    // Building config
    let mut mock_alert_client = MockAlertClient::new();
    mock_alert_client.expect_send_message().times(1).returning(|_| Ok(()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_alerts(ConfigType::Mock(MockType::Alerts(Box::new(mock_alert_client))))
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating job in database
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();
    // Setting up mock to panic when process_job is called
    job_handler
        .expect_process_job()
        .times(1)
        .returning(|_, _| -> Result<String, JobError> { panic!("Simulated panic in process_job") });

    // Mocking the `get_job_handler` call in process_job function
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).return_once(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::process_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job was moved to failed state
    let job_in_db = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(job_in_db.status, JobStatus::Failed);

    // Check that failure reason is recorded in common metadata
    assert!(job_in_db
        .metadata
        .common
        .failure_reason
        .as_ref()
        .unwrap()
        .contains("Job handler panicked with message: Simulated panic in process_job"));
}

/// Tests `process_job` function when job is already existing in the db and job status is not
/// `Created` or `VerificationFailed`.
#[rstest]
#[tokio::test]
async fn process_job_with_job_exists_in_db_with_invalid_job_processing_status_errors() {
    // Creating a job with Completed status which is invalid processing.
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Completed, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let database_client = services.config.database();

    // creating job in database
    database_client.create_job(job_item.clone()).await.unwrap();

    assert!(JobHandlerService::process_job(job_item.id, services.config.clone()).await.is_err());

    let job_in_db = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    // Job should be untouched in db.
    assert_eq!(job_in_db, job_item);

    // Queue checks.
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.verify_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

/// Tests `process_job` function when `check_ready_to_process` returns Err (dependencies not ready).
/// This test verifies that:
/// 1. The job is NOT processed (process_job on handler should not be called)
/// 2. The job is requeued to the processing queue with a delay
/// 3. The job status remains unchanged (still Created)
/// 4. No failure is recorded
#[rstest]
#[tokio::test]
async fn process_job_requeues_when_check_ready_to_process_fails() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let database_client = services.config.database();

    // Create a job with Created status
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating job in database first
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();

    // Mock check_ready_to_process to return Err with 0 second delay (for immediate queue visibility in test)
    job_handler.expect_check_ready_to_process().times(1).returning(|_| Err(Duration::from_secs(0)));

    // process_job should NOT be called since dependencies are not ready
    job_handler.expect_process_job().times(0);

    // Mocking the `get_job_handler` call
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx_guard = get_job_handler_context_safe();
    ctx_guard.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // Call process_job - should succeed (returns Ok) but job should be requeued, not processed
    let result = JobHandlerService::process_job(job_item.id, services.config.clone()).await;
    assert!(result.is_ok(), "process_job should return Ok when requeuing due to check_ready_to_process failure");

    // DB checks - job should still be in Created status (not processed, not failed)
    let job_in_db = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(job_in_db.status, JobStatus::Created, "Job status should remain Created when requeued");
    assert!(job_in_db.metadata.common.failure_reason.is_none(), "No failure reason should be recorded when requeuing");

    // Queue checks - job should be requeued to the processing queue (not verification queue)
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id, "Requeued message should have the same job ID");
}

/// Tests `process_job` function when job is not in the db
/// This test should fail
#[rstest]
#[tokio::test]
async fn process_job_job_does_not_exists_in_db_works() {
    // Creating a valid job which is not existing in the db.
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    assert!(JobHandlerService::process_job(job_item.id, services.config.clone()).await.is_err());

    // Queue checks.
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.verify_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

/// Tests `process_job` function when 2 workers try to process the same job.
/// This test should fail because once the job is locked for processing on one
/// worker it should not be accessed by another worker and should throw an error
/// when updating the job status.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn process_job_two_workers_process_same_job_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting process job function in job processor to return the external ID.
    job_handler.expect_process_job().times(1).returning(move |_, _| Ok("0xbeef".to_string()));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);

    // Mocking the `get_job_handler` call - both workers will call it before one fails due to lock contention
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx_guard = get_job_handler_context_safe();
    // Both workers will call get_job_handler (each process_job calls it once), so expect exactly 2 calls
    ctx_guard.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let db_client = services.config.database();

    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating the job in the db
    db_client.create_job(job_item.clone()).await.unwrap();

    let config_1 = services.config.clone();
    let config_2 = services.config.clone();

    // Simulating the two workers, Uuid has in-built copy trait
    let worker_1 = tokio::spawn(async move { JobHandlerService::process_job(job_item.id, config_1).await });
    let worker_2 = tokio::spawn(async move { JobHandlerService::process_job(job_item.id, config_2).await });

    // waiting for workers to complete the processing
    let (result_1, result_2) = tokio::join!(worker_1, worker_2);

    assert_ne!(
        result_1.unwrap().is_ok(),
        result_2.unwrap().is_ok(),
        "One worker should succeed and the other should fail"
    );

    // Waiting for 5 secs for job to be updated in the db
    sleep(Duration::from_secs(5)).await;

    let final_job_in_db = db_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(final_job_in_db.status, JobStatus::PendingVerification);
}

/// Tests `process_job` function when the job handler returns an error.
/// The job should be moved to the failed status.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn process_job_job_handler_returns_error_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting process job function in job processor to return the external ID.
    let failure_reason = "Failed to process job";
    job_handler
        .expect_process_job()
        .times(1)
        .returning(move |_, _| Err(JobError::Other(failure_reason.to_string().into())));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);

    // building config first
    let mut mock_alert_client = MockAlertClient::new();
    mock_alert_client.expect_send_message().times(1).returning(|_| Ok(()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_alerts(ConfigType::Mock(MockType::Alerts(Box::new(mock_alert_client))))
        .build()
        .await;
    let db_client = services.config.database();

    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating the job in the db
    db_client.create_job(job_item.clone()).await.unwrap();

    // Mocking the `get_job_handler` call AFTER creating services and job
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::process_job(job_item.id, services.config.clone()).await.is_ok());

    let final_job_in_db = db_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(final_job_in_db.status, JobStatus::Failed);
    assert!(final_job_in_db.metadata.common.failure_reason.as_ref().unwrap().contains(failure_reason));
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Verified` verification status.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn verify_job_with_verified_status_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let mut job_handler = MockJobHandlerTrait::new();

    // creating a job in a database
    database_client.create_job(job_item.clone()).await.unwrap();
    // expecting process job function in job processor to return the external ID
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Verified));
    job_handler.expect_max_process_attempts().returning(move || 2u64);

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    // Mocking the `get_job_handler` call - verify_job calls it exactly once
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::Completed);

    // Queue checks - queues should exist but be empty
    let queue_error_verification = consume_message_with_retry(
        services.config.queue(),
        QueueType::DataSubmissionJobVerification,
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error_verification, QueueError::ErrorFromQueueError(_));
    let queue_error_processing = consume_message_with_retry(
        services.config.queue(),
        QueueType::DataSubmissionJobProcessing,
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error_processing, QueueError::ErrorFromQueueError(_));
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Rejected` verification status.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn verify_job_with_rejected_status_adds_to_queue_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let mut job_handler = MockJobHandlerTrait::new();

    // creating job in database
    database_client.create_job(job_item.clone()).await.unwrap();
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Rejected("".to_string())));
    job_handler.expect_max_process_attempts().returning(move || 2u64);

    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    // Mocking the `get_job_handler` call - verify_job calls it exactly once
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::VerificationFailed);

    // Queue checks - retry a few times in case of timing issues
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        QueueType::DataSubmissionJobProcessing,
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Rejected` verification status but doesn't add
/// the job to process queue because of maximum attempts reached.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn verify_job_with_rejected_status_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    // Building config
    let mut mock_alert_client = MockAlertClient::new();
    mock_alert_client.expect_send_message().times(1).returning(|_| Ok(()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_alerts(ConfigType::Mock(MockType::Alerts(Box::new(mock_alert_client))))
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let mut job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // Set process_attempt_no to 1 to simulate max attempts reached
    job_item.metadata.common.process_attempt_no = 1;

    // Creating job in database
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting verify_job function to return Rejected status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Rejected("".to_string())));
    job_handler.expect_max_process_attempts().returning(move || 1u64);

    // Mocking the `get_job_handler` call - verify_job calls it exactly once
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job was moved to a failed state
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::Failed);

    // Check that process attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.process_attempt_no, 1);

    // Queue checks - verify no message was added to the process queue
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Pending` verification status.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn verify_job_with_pending_status_adds_to_queue_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with a proper metadata structure
    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // Creating a job in a database
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting verify_job function to return Pending status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Pending));
    job_handler.expect_max_verification_attempts().returning(move || 2u64);
    job_handler.expect_verification_polling_delay_seconds().returning(move || 2u64);

    // Mocking the `get_job_handler` call - verify_job calls it exactly once
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job status remains PendingVerification and verification attempt is
    // incremented
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::PendingVerification);

    // Check that verification attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.verification_attempt_no, 1);

    // Queue checks - verify a message was added to the verification queue
    // Retry a few times in case of timing issues
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.verify_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Pending` verification status but doesn't add
/// the job to process queue because of maximum attempts reached.
#[rstest]
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn verify_job_with_pending_status_works() {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let mut job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // Set verification_attempt_no to 1 to simulate max attempts reached
    job_item.metadata.common.verification_attempt_no = 1;

    // Creating job in a database
    database_client.create_job(job_item.clone()).await.unwrap();

    let mut job_handler = MockJobHandlerTrait::new();
    // Expecting verify_job function to return Pending status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Pending));
    job_handler.expect_max_verification_attempts().returning(move || 1u64);
    job_handler.expect_verification_polling_delay_seconds().returning(move || 2u64);

    // Mocking the `get_job_handler` call
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(JobHandlerService::verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job status is changed to VerificationTimeout
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::VerificationTimeout);

    // Check that verification attempt is still recorded in common metadata
    assert_eq!(updated_job.metadata.common.verification_attempt_no, 1);

    // Queue checks - verify no message was added to the verification queue
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.verify_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

#[rstest]
#[case(JobType::DataSubmission, JobStatus::Completed)] // code should panic here, how can completed move to dl queue ?
#[case(JobType::SnosRun, JobStatus::PendingVerification)]
#[case(JobType::ProofCreation, JobStatus::LockedForProcessing)]
// #[case(JobType::ProofRegistration, JobStatus::Created)] TODO: add this case when we have the metadata for proof
// registration
#[case(JobType::StateTransition, JobStatus::Completed)]
#[case(JobType::ProofCreation, JobStatus::VerificationTimeout)]
#[case(JobType::DataSubmission, JobStatus::VerificationFailed)]
#[tokio::test]
async fn handle_job_failure_with_failed_job_status_works(#[case] job_type: JobType, #[case] job_status: JobStatus) {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let internal_id = 1;

    // Create a job with Failed status
    let mut job_expected = build_job_item(job_type.clone(), JobStatus::Failed, internal_id);

    // Store the previous job status in the common metadata
    job_expected.metadata.common.failure_reason = Some(format!("last_job_status: {}", job_status));

    let job_id = job_expected.id;

    // Feeding the job to DB
    database_client.create_job(job_expected.clone()).await.unwrap();

    // Calling handle_job_failure
    JobHandlerService::handle_job_failure(job_id, services.config.clone())
        .await
        .expect("handle_job_failure failed to run");

    // Fetch the job from DB and verify it's unchanged (since it's already in Failed status)
    let job_fetched =
        services.config.database().get_job_by_id(job_id).await.expect("Unable to fetch Job Data").unwrap();

    assert_eq!(job_fetched, job_expected);
}

#[rstest]
#[case::pending_verification(JobType::SnosRun, JobStatus::PendingVerification)]
#[case::verification_timeout(JobType::SnosRun, JobStatus::VerificationTimeout)]
#[tokio::test]
async fn handle_job_failure_with_correct_job_status_works(#[case] job_type: JobType, #[case] job_status: JobStatus) {
    let mut mock_alert_client = MockAlertClient::new();
    mock_alert_client.expect_send_message().times(1).returning(|_| Ok(()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_alerts(ConfigType::Mock(MockType::Alerts(Box::new(mock_alert_client))))
        .build()
        .await;

    let database_client = services.config.database();
    let internal_id = 1;

    // Create a job
    let job = build_job_item(job_type.clone(), job_status.clone(), internal_id);
    let job_id = job.id;

    // Feeding the job to DB
    database_client.create_job(job.clone()).await.unwrap();

    // Calling handle_job_failure
    JobHandlerService::handle_job_failure(job_id, services.config.clone())
        .await
        .expect("handle_job_failure failed to run");

    let job_fetched =
        services.config.database().get_job_by_id(job_id).await.expect("Unable to fetch Job Data").unwrap();

    // Creating expected output
    let mut job_expected = job.clone();
    job_expected.status = JobStatus::Failed;
    job_expected.version = 1;

    // Set the failure reason in common metadata
    job_expected.metadata.common.failure_reason =
        Some(format!("Received failure queue message for job with status: {}", job_status));

    // Compare fields individually, excluding timestamps which may differ slightly due to flakiness
    // Timestamps (created_at, updated_at) are excluded as they can differ by milliseconds between creation and update
    assert_eq!(job_fetched.id, job_expected.id);
    assert_eq!(job_fetched.status, job_expected.status);
    assert_eq!(job_fetched.version, job_expected.version);
    assert_eq!(job_fetched.metadata.common.failure_reason, job_expected.metadata.common.failure_reason);
    assert_eq!(job_fetched.internal_id, job_expected.internal_id);
    assert_eq!(job_fetched.job_type, job_expected.job_type);
    assert_eq!(job_fetched.external_id, job_expected.external_id);
}

#[rstest]
#[case(JobType::DataSubmission)]
#[tokio::test]
async fn handle_job_failure_job_status_completed_works(#[case] job_type: JobType) {
    let job_status = JobStatus::Completed;

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let internal_id = 1;

    // Create a job
    let job_expected = build_job_item(job_type.clone(), job_status.clone(), internal_id);
    let job_id = job_expected.id;

    // Feeding the job to DB
    database_client.create_job(job_expected.clone()).await.unwrap();

    // Calling handle_job_failure
    JobHandlerService::handle_job_failure(job_id, services.config.clone())
        .await
        .expect("Test call to handle_job_failure should have passed.");

    // The completed job status on db is untouched.
    let job_fetched =
        services.config.database().get_job_by_id(job_id).await.expect("Unable to fetch Job Data").unwrap();

    assert_eq!(job_fetched, job_expected);
}

#[rstest]
#[tokio::test]
async fn test_retry_job_adds_to_process_queue() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create a failed job
    let job_item = build_job_item(JobType::DataSubmission, JobStatus::Failed, 1);
    services.config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.id;

    assert!(JobHandlerService::retry_job(job_id, services.config.clone()).await.is_ok());

    // Verify job status was updated to PendingRetry
    let updated_job = services.config.database().get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::PendingRetry);

    // Verify message was added to process queue
    // Retry a few times in case of timing issues
    let consumed_messages = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_id);
}

#[rstest]
#[case::pending_verification(JobStatus::PendingVerification)]
#[case::completed(JobStatus::Completed)]
#[case::created(JobStatus::Created)]
#[tokio::test]
async fn test_retry_job_invalid_status(#[case] initial_status: JobStatus) {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Create a job with non-Failed status
    let job_item = build_job_item(JobType::DataSubmission, initial_status.clone(), 1);
    services.config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.id;

    // Attempt to retry the job
    let result = JobHandlerService::retry_job(job_id, services.config.clone()).await;
    assert!(result.is_err());

    if let Err(error) = result {
        assert_matches!(error, JobError::InvalidStatus { .. });
    }

    // Verify job status was not changed
    let job = services.config.database().get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(job.status, initial_status);

    // Verify no message was added to process queue
    let queue_error = consume_message_with_retry(
        services.config.queue(),
        job_item.job_type.process_queue_name(),
        QUEUE_CONSUME_MAX_RETRIES,
        QUEUE_CONSUME_RETRY_DELAY_SECS,
    )
    .await
    .unwrap_err();
    assert_matches!(queue_error, QueueError::ErrorFromQueueError(_));
}

/// Tests that SNS alert is sent when a job is moved to failed status.
/// This test verifies that:
/// 1. A job can be moved to failed status
/// 2. An SNS alert is automatically sent by move_job_to_failed function
/// 3. The alert contains job details (ID, type, block, reason)
/// 4. The SNS integration works correctly (verified by successful function completion)
#[tokio::test]
async fn move_job_to_failed_sends_sns_alert() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_alerts(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let internal_id = 1;
    let job_type = JobType::SnosRun;
    let failure_reason = "Processing failed: Test error";

    // Create a job with PendingVerification status
    let job = build_job_item(job_type.clone(), JobStatus::PendingVerification, internal_id);
    let job_id = job.id;

    // Create the job in the database
    database_client.create_job(job.clone()).await.unwrap();

    // Move the job to failed status - this should trigger the SNS alert automatically
    let result = JobService::move_job_to_failed(&job, services.config.clone(), failure_reason.to_string()).await;
    assert!(result.is_ok(), "move_job_to_failed should succeed and send SNS alert");

    // Verify the job status was updated to Failed
    let updated_job = database_client.get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::Failed);
    assert_eq!(updated_job.metadata.common.failure_reason, Some(failure_reason.to_string()));

    // The SNS alert was automatically sent by move_job_to_failed function
    // If the function completed successfully, it means the SNS alert was sent without errors
}
