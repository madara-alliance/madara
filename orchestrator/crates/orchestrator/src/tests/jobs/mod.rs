use std::sync::Arc;
use std::time::Duration;

use mockall::predicate::eq;
use mongodb::bson::doc;
use omniqueue::QueueError;
use rstest::rstest;
use tokio::time::sleep;

use crate::constants::CAIRO_PIE_FILE_NAME;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputTypePath, ProvingMetadata, SnosMetadata,
};
use crate::jobs::types::{ExternalId, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::{create_job, handle_job_failure, process_job, retry_job, verify_job, Job, JobError, MockJob};
use crate::queue::job_queue::QueueNameForJobType;
use crate::queue::QueueType;
use crate::tests::common::MessagePayloadType;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::build_job_item;

#[cfg(test)]
pub mod da_job;

#[cfg(test)]
pub mod proving_job;

#[cfg(test)]
pub mod state_update_job;

#[cfg(test)]
pub mod snos_job;

use assert_matches::assert_matches;

/// Tests `create_job` function when job is not existing in the db.
#[rstest]
#[tokio::test]
async fn create_job_job_does_not_exists_in_db_works() {
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 0);
    let mut job_handler = MockJob::new();

    // Adding expectation for creation of new job.
    let job_item_clone = job_item.clone();
    job_handler.expect_create_job().times(1).returning(move |_, _, _| Ok(job_item_clone.clone()));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Mocking the `get_job_handler` call in create_job function.
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).return_once(move |_| Arc::clone(&job_handler));

    // Create a proper JobMetadata for the test
    let metadata =
        JobMetadata { common: CommonMetadata::default(), specific: JobSpecificMetadata::Snos(SnosMetadata::default()) };

    assert!(create_job(JobType::SnosRun, "0".to_string(), metadata, services.config.clone()).await.is_ok());

    // Db checks.
    let job_in_db = services.config.database().get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(job_in_db.id, job_item.id);
    assert_eq!(job_in_db.internal_id, job_item.internal_id);
    assert_eq!(job_in_db.metadata, job_item.metadata);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.process_queue_name()).await.unwrap();
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
    database_client.create_job_item(job_item.clone()).await.unwrap();

    // Create a proper JobMetadata for the test
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputTypePath::CairoPie(format!("{}/{}", "0", CAIRO_PIE_FILE_NAME))),
            ..Default::default()
        }),
    };

    assert!(create_job(JobType::ProofCreation, "0".to_string(), metadata, services.config.clone()).await.is_ok());

    // There should be only 1 job in the db
    let jobs_in_db = database_client.get_jobs_by_statuses(vec![JobStatus::Created], None).await.unwrap();
    assert_eq!(jobs_in_db.len(), 1);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.process_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages, QueueError::NoData);
}

/// Tests `create_job` function when job handler is not implemented in the `get_job_handler`
/// This test should fail as job handler is not implemented in the `factory.rs`
#[rstest]
#[should_panic(expected = "Job type not implemented yet.")]
#[tokio::test]
async fn create_job_job_handler_is_not_implemented_panics() {
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    // Mocking the `get_job_handler` call in create_job function.
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).returning(|_| panic!("Job type not implemented yet."));

    let job_type = JobType::ProofCreation;

    // Create a proper JobMetadata for the test
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputTypePath::CairoPie(format!("{}/{}", "0", CAIRO_PIE_FILE_NAME))),
            ..Default::default()
        }),
    };

    assert!(create_job(job_type.clone(), "0".to_string(), metadata, services.config.clone()).await.is_err());

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_type.process_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages, QueueError::NoData);
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
    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let job_item = build_job_item(job_type.clone(), job_status.clone(), 1);

    let mut job_handler = MockJob::new();

    // Creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();
    // Expecting process job function in job processor to return the external ID.
    job_handler.expect_process_job().times(1).returning(move |_, _| Ok("0xbeef".to_string()));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    job_handler.expect_job_processing_lock().return_const(None);

    // Mocking the `get_job_handler` call in create_job function.
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(job_type.clone())).returning(move |_| Arc::clone(&job_handler));

    assert!(process_job(job_item.id, services.config.clone()).await.is_ok());
    // Getting the updated job.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    // checking if job_status is updated in db
    assert_eq!(updated_job.status, JobStatus::PendingVerification);
    assert_eq!(updated_job.external_id, ExternalId::String(Box::from("0xbeef")));

    // Check that process attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.process_attempt_no, 1);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_type.verify_queue_name()).await.unwrap();
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
async fn process_job_handles_panic() {
    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();

    let mut job_handler = MockJob::new();
    // Setting up mock to panic when process_job is called
    job_handler
        .expect_process_job()
        .times(1)
        .returning(|_, _| -> Result<String, JobError> { panic!("Simulated panic in process_job") });
    job_handler.expect_job_processing_lock().return_const(None);

    // Mocking the `get_job_handler` call in process_job function
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).return_once(move |_| Arc::clone(&job_handler));

    assert!(process_job(job_item.id, services.config.clone()).await.is_ok());

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
    database_client.create_job_item(job_item.clone()).await.unwrap();

    assert!(process_job(job_item.id, services.config.clone()).await.is_err());

    let job_in_db = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    // Job should be untouched in db.
    assert_eq!(job_in_db, job_item);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.verify_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages, QueueError::NoData);
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

    assert!(process_job(job_item.id, services.config.clone()).await.is_err());

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.verify_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages, QueueError::NoData);
}

/// Tests `process_job` function when 2 workers try to process the same job.
/// This test should fail because once the job is locked for processing on one
/// worker it should not be accessed by another worker and should throw an error
/// when updating the job status.
#[rstest]
#[tokio::test]
async fn process_job_two_workers_process_same_job_works() {
    let mut job_handler = MockJob::new();
    // Expecting process job function in job processor to return the external ID.
    job_handler.expect_process_job().times(1).returning(move |_, _| Ok("0xbeef".to_string()));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    job_handler.expect_job_processing_lock().return_const(None);

    // Mocking the `get_job_handler` call in create_job function.
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let db_client = services.config.database();

    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating the job in the db
    db_client.create_job_item(job_item.clone()).await.unwrap();

    let config_1 = services.config.clone();
    let config_2 = services.config.clone();

    // Simulating the two workers, Uuid has in-built copy trait
    let worker_1 = tokio::spawn(async move { process_job(job_item.id, config_1).await });
    let worker_2 = tokio::spawn(async move { process_job(job_item.id, config_2).await });

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
async fn process_job_job_handler_returns_error_works() {
    let mut job_handler = MockJob::new();
    // Expecting process job function in job processor to return the external ID.
    let failure_reason = "Failed to process job";
    job_handler
        .expect_process_job()
        .times(1)
        .returning(move |_, _| Err(JobError::Other(failure_reason.to_string().into())));
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    job_handler.expect_job_processing_lock().return_const(None);

    // Mocking the `get_job_handler` call in create_job function.
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::SnosRun)).returning(move |_| Arc::clone(&job_handler));

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;
    let db_client = services.config.database();

    let job_item = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    // Creating the job in the db
    db_client.create_job_item(job_item.clone()).await.unwrap();

    assert!(process_job(job_item.id, services.config.clone()).await.is_ok());

    let final_job_in_db = db_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(final_job_in_db.status, JobStatus::Failed);
    assert!(final_job_in_db.metadata.common.failure_reason.as_ref().unwrap().contains(failure_reason));
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Verified` verification status.
#[rstest]
#[tokio::test]
async fn verify_job_with_verified_status_works() {
    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let mut job_handler = MockJob::new();

    // creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();
    // expecting process job function in job processor to return the external ID
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Verified));
    job_handler.expect_max_process_attempts().returning(move || 2u16);

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    // Mocking the `get_job_handler` call in create_job function.
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::Completed);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages_verification_queue =
        services.config.queue().consume_message_from_queue(QueueType::DataSubmissionJobVerification).await.unwrap_err();
    assert_matches!(consumed_messages_verification_queue, QueueError::NoData);
    let consumed_messages_processing_queue =
        services.config.queue().consume_message_from_queue(QueueType::DataSubmissionJobProcessing).await.unwrap_err();
    assert_matches!(consumed_messages_processing_queue, QueueError::NoData);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Rejected` verification status.
#[rstest]
#[tokio::test]
async fn verify_job_with_rejected_status_adds_to_queue_works() {
    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let mut job_handler = MockJob::new();

    // creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Rejected("".to_string())));
    job_handler.expect_max_process_attempts().returning(move || 2u16);

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    // Mocking the `get_job_handler` call in create_job function.
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks.
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::VerificationFailed);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks.
    let consumed_messages =
        services.config.queue().consume_message_from_queue(QueueType::DataSubmissionJobProcessing).await.unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Rejected` verification status but doesn't add
/// the job to process queue because of maximum attempts reached.
#[rstest]
#[tokio::test]
async fn verify_job_with_rejected_status_works() {
    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let mut job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // Set process_attempt_no to 1 to simulate max attempts reached
    job_item.metadata.common.process_attempt_no = 1;

    // Creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();

    let mut job_handler = MockJob::new();
    // Expecting verify_job function to return Rejected status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Rejected("".to_string())));
    job_handler.expect_max_process_attempts().returning(move || 1u16);

    // Mocking the `get_job_handler` call
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job was moved to failed state
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::Failed);

    // Check that process attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.process_attempt_no, 1);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks - verify no message was added to the process queue
    let consumed_messages_processing_queue =
        services.config.queue().consume_message_from_queue(job_item.job_type.process_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages_processing_queue, QueueError::NoData);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Pending` verification status.
#[rstest]
#[tokio::test]
async fn verify_job_with_pending_status_adds_to_queue_works() {
    // Building config
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();

    // Create a job with proper metadata structure
    let job_item = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 1);

    // Creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();

    let mut job_handler = MockJob::new();
    // Expecting verify_job function to return Pending status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Pending));
    job_handler.expect_max_verification_attempts().returning(move || 2u16);
    job_handler.expect_verification_polling_delay_seconds().returning(move || 2u64);

    // Mocking the `get_job_handler` call
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job status remains PendingVerification and verification attempt is
    // incremented
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::PendingVerification);

    // Check that verification attempt is recorded in common metadata
    assert_eq!(updated_job.metadata.common.verification_attempt_no, 1);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks - verify a message was added to the verification queue
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.verify_queue_name()).await.unwrap();
    let consumed_message_payload: MessagePayloadType = consumed_messages.payload_serde_json().unwrap().unwrap();
    assert_eq!(consumed_message_payload.id, job_item.id);
}

/// Tests `verify_job` function when job is having expected status
/// and returns a `Pending` verification status but doesn't add
/// the job to process queue because of maximum attempts reached.
#[rstest]
#[tokio::test]
async fn verify_job_with_pending_status_works() {
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

    // Creating job in database
    database_client.create_job_item(job_item.clone()).await.unwrap();

    let mut job_handler = MockJob::new();
    // Expecting verify_job function to return Pending status
    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Pending));
    job_handler.expect_max_verification_attempts().returning(move || 1u16);
    job_handler.expect_verification_polling_delay_seconds().returning(move || 2u64);

    // Mocking the `get_job_handler` call
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(JobType::DataSubmission)).returning(move |_| Arc::clone(&job_handler));

    assert!(verify_job(job_item.id, services.config.clone()).await.is_ok());

    // DB checks - verify the job status is changed to VerificationTimeout
    let updated_job = database_client.get_job_by_id(job_item.id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::VerificationTimeout);

    // Check that verification attempt is still recorded in common metadata
    assert_eq!(updated_job.metadata.common.verification_attempt_no, 1);

    // Waiting for 5 secs for message to be passed into the queue
    sleep(Duration::from_secs(5)).await;

    // Queue checks - verify no message was added to the verification queue
    let consumed_messages_verification_queue =
        services.config.queue().consume_message_from_queue(job_item.job_type.verify_queue_name()).await.unwrap_err();
    assert_matches!(consumed_messages_verification_queue, QueueError::NoData);
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
    database_client.create_job_item(job_expected.clone()).await.unwrap();

    // Calling handle_job_failure
    handle_job_failure(job_id, services.config.clone()).await.expect("handle_job_failure failed to run");

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
    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .build()
        .await;

    let database_client = services.config.database();
    let internal_id = 1;

    // Create a job
    let job = build_job_item(job_type.clone(), job_status.clone(), internal_id);
    let job_id = job.id;

    // Feeding the job to DB
    database_client.create_job_item(job.clone()).await.unwrap();

    // Calling handle_job_failure
    handle_job_failure(job_id, services.config.clone()).await.expect("handle_job_failure failed to run");

    let job_fetched =
        services.config.database().get_job_by_id(job_id).await.expect("Unable to fetch Job Data").unwrap();

    // Creating expected output
    let mut job_expected = job.clone();
    job_expected.status = JobStatus::Failed;
    job_expected.version = 1;

    // Set the failure reason in common metadata
    job_expected.metadata.common.failure_reason =
        Some(format!("Received failure queue message for job with status: {}", job_status));

    assert_eq!(job_fetched, job_expected);
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
    database_client.create_job_item(job_expected.clone()).await.unwrap();

    // Calling handle_job_failure
    handle_job_failure(job_id, services.config.clone())
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
    services.config.database().create_job_item(job_item.clone()).await.unwrap();
    let job_id = job_item.id;

    // Retry the job
    assert!(retry_job(job_id, services.config.clone()).await.is_ok());

    // Verify job status was updated to PendingRetry
    let updated_job = services.config.database().get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(updated_job.status, JobStatus::PendingRetry);

    // Wait for message to be processed
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify message was added to process queue
    let consumed_messages =
        services.config.queue().consume_message_from_queue(job_item.job_type.process_queue_name()).await.unwrap();

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
    services.config.database().create_job_item(job_item.clone()).await.unwrap();
    let job_id = job_item.id;

    // Attempt to retry the job
    let result = retry_job(job_id, services.config.clone()).await;
    assert!(result.is_err());

    if let Err(error) = result {
        assert_matches!(error, JobError::InvalidStatus { .. });
    }

    // Verify job status was not changed
    let job = services.config.database().get_job_by_id(job_id).await.unwrap().unwrap();
    assert_eq!(job.status, initial_status);

    // Wait briefly to ensure no messages were added
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Verify no message was added to process queue
    let queue_result = services.config.queue().consume_message_from_queue(job_item.job_type.process_queue_name()).await;
    assert_matches!(queue_result, Err(QueueError::NoData));
}
