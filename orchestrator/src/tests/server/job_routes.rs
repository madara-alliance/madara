use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hyper::{Body, Request};
use mockall::predicate::eq;
use orchestrator_utils::env_utils::get_env_var_or_panic;
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

use crate::core::config::Config;
use crate::server::types::ApiResponse;
use crate::tests::common::consume_message_with_retry;
use crate::tests::common::mock_helpers::{acquire_test_lock, get_job_handler_context_safe};
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::build_job_item;
use crate::types::jobs::metadata::{JobSpecificMetadata, SettlementContext};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueNameForJobType;
use crate::worker::event_handler::jobs::{JobHandlerTrait, MockJobHandlerTrait};
use crate::worker::parser::job_queue_message::JobQueueMessage;

#[fixture]
async fn setup_trigger() -> (SocketAddr, Arc<Config>) {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env.test file");

    let madara_url = get_env_var_or_panic("MADARA_ORCHESTRATOR_MADARA_RPC_URL");
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(madara_url.as_str().to_string().as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_starknet_client(provider.into())
        .configure_api_server(ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();
    let config = services.config;
    (addr, config)
}

#[tokio::test]
#[rstest]
#[allow(clippy::await_holding_lock)]
async fn test_trigger_process_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), JobStatus::Created, 1);
    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    // Set up mock job handler - queue_job_for_processing doesn't call get_job_handler, so times(0)
    let mut job_handler = MockJobHandlerTrait::new();
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().with(eq(job_type.clone())).times(0).returning(move |_| Arc::clone(&job_handler));

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder().uri(format!("http://{}/jobs/{}/process", addr, job_id)).body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();

    // Verify response status and message
    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);
    assert_eq!(response.message, Some(format!("Job with id {} queued for processing", job_id)));

    // Wait a bit for queue message to be available
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify job was added to process queue - retry a few times in case of timing issues
    let queue_message = consume_message_with_retry(config.queue(), job_type.process_queue_name(), 5, 1).await;
    let message_payload: JobQueueMessage = queue_message.payload_serde_json().unwrap().unwrap();
    assert_eq!(message_payload.id, job_id);

    // Verify job status and metadata
    if let Some(job_fetched) = config.database().get_job_by_id(job_id).await.unwrap() {
        assert_eq!(job_fetched.id, job_item.id);
        assert_eq!(job_fetched.status, JobStatus::Created);
    } else {
        panic!("Could not get job from database")
    }
}

#[tokio::test]
#[rstest]
#[allow(clippy::await_holding_lock)]
async fn test_trigger_verify_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    // Acquire test lock to serialize this test with others that use mocks
    let _test_lock = acquire_test_lock();

    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    // Create a job with initial metadata
    let mut job_item = build_job_item(job_type.clone(), JobStatus::PendingVerification, 1);

    // Set verification counters in common metadata
    job_item.metadata.common.verification_retry_attempt_no = 0;
    job_item.metadata.common.verification_attempt_no = 10;

    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    // Set up mock job handler
    let mut job_handler = MockJobHandlerTrait::new();
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));

    let ctx = get_job_handler_context_safe();
    ctx.expect().with(eq(job_type.clone())).times(1).returning(move |_| Arc::clone(&job_handler));

    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/jobs/{}/verify", addr, job_id)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);
    assert_eq!(response.message, Some(format!("Job with id {} queued for verification", job_id)));

    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify job was added to verification queue - retry a few times in case of timing issues
    let queue_message = consume_message_with_retry(config.queue(), job_type.verify_queue_name(), 5, 1).await;
    let message_payload: JobQueueMessage = queue_message.payload_serde_json().unwrap().unwrap();
    assert_eq!(message_payload.id, job_id);

    // Verify job status and metadata
    let job_fetched = config.database().get_job_by_id(job_id).await.unwrap().expect("Could not get job from database");
    assert_eq!(job_fetched.id, job_item.id);
    assert_eq!(job_fetched.status, JobStatus::PendingVerification);

    // Verify verification attempt was reset
    assert_eq!(job_fetched.metadata.common.verification_attempt_no, 0);

    // Verify retry attempt was incremented
    assert_eq!(job_fetched.metadata.common.verification_retry_attempt_no, 1);
}

#[tokio::test]
#[rstest]
async fn test_trigger_retry_job_when_failed(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), JobStatus::Failed, 1);
    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    // Set up mock job handler - retry_job doesn't call get_job_handler directly, so times(0)
    let mut job_handler = MockJobHandlerTrait::new();
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().with(eq(job_type.clone())).times(0).returning(move |_| Arc::clone(&job_handler));

    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/jobs/{}/retry", addr, job_id)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);
    assert_eq!(response.message, Some(format!("Job with id {} retry initiated", job_id)));

    // Wait a bit for queue message to be available
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Verify job was added to process queue - retry a few times in case of timing issues
    let queue_message = consume_message_with_retry(config.queue(), job_type.process_queue_name(), 5, 1).await;
    let message_payload: JobQueueMessage = queue_message.payload_serde_json().unwrap().unwrap();
    assert_eq!(message_payload.id, job_id);

    // Verify job status changed to PendingRetry
    let job_fetched = config.database().get_job_by_id(job_id).await.unwrap().expect("Could not get job from database");
    assert_eq!(job_fetched.id, job_item.id);
    assert_eq!(job_fetched.metadata.common.process_retry_attempt_no, 1);
    assert_eq!(job_fetched.status, JobStatus::PendingRetry);
}

#[rstest]
#[case::pending_verification_job(JobStatus::PendingVerification)]
#[case::completed_job(JobStatus::Completed)]
#[case::created_job(JobStatus::Created)]
#[tokio::test]
async fn test_trigger_retry_job_not_allowed(
    #[future] setup_trigger: (SocketAddr, Arc<Config>),
    #[case] initial_status: JobStatus,
) {
    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), initial_status.clone(), 1);
    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/jobs/{}/retry", addr, job_id)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    // Verify request was rejected
    assert_eq!(response.status(), 400);

    // Verify job status hasn't changed
    let job_fetched = config.database().get_job_by_id(job_id).await.unwrap().expect("Could not get job from database");
    assert_eq!(job_fetched.status, initial_status);

    // Verify no message was added to the queue
    let queue_result = config.queue().consume_message_from_queue(job_type.process_queue_name()).await;
    assert!(queue_result.is_err(), "Queue should be empty - no message should be added for non-Failed jobs");
}

#[tokio::test]
#[rstest]
async fn test_get_job_status_by_block_number_found(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let block_number = 123;

    // Create some jobs for the block
    let mut snos_job = build_job_item(JobType::SnosRun, JobStatus::Completed, 1); // internal_id is not block_number for SnosRun jobs
    if let JobSpecificMetadata::Snos(ref mut x) = snos_job.metadata.specific {
        x.start_block = block_number - 1;
        x.end_block = block_number + 1;
    }

    let proving_job = build_job_item(JobType::ProofCreation, JobStatus::PendingVerification, block_number);
    let data_submission_job = build_job_item(JobType::DataSubmission, JobStatus::Created, block_number);

    let state_transition_job = build_job_item(JobType::StateTransition, JobStatus::Completed, 1); // internal_id is not block_number for ST
    let mut state_transition_job_specific_metadata = state_transition_job.metadata.specific.clone();
    if let JobSpecificMetadata::StateUpdate(ref mut x) = state_transition_job_specific_metadata {
        if let SettlementContext::Block(ref mut y) = x.context {
            y.to_settle = vec![block_number, block_number + 1];
        } else {
            panic!("Unexpected settlement context");
        }
    } else {
        panic!("Unexpected job type");
    }
    let mut state_transition_job_updated = state_transition_job.clone();
    state_transition_job_updated.metadata.specific = state_transition_job_specific_metadata;

    config.database().create_job(snos_job.clone()).await.unwrap();
    config.database().create_job(proving_job.clone()).await.unwrap();
    config.database().create_job(data_submission_job.clone()).await.unwrap();
    config.database().create_job(state_transition_job_updated.clone()).await.unwrap();

    // Create a job for a different block to ensure it's not returned
    let other_block_job = build_job_item(JobType::SnosRun, JobStatus::Completed, block_number + 10);
    config.database().create_job(other_block_job).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/jobs/block/{}/status", addr, block_number))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<crate::server::types::JobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(response_body.message, Some(format!("Successfully fetched job statuses for block {}", block_number)));
    let jobs_response = response_body.data.unwrap().jobs;

    assert_eq!(jobs_response.len(), 4);

    // Check that the correct jobs are returned
    assert!(jobs_response.iter().any(|j| j.id == snos_job.id && j.status == JobStatus::Completed));
    assert!(jobs_response.iter().any(|j| j.id == proving_job.id && j.status == JobStatus::PendingVerification));
    assert!(jobs_response.iter().any(|j| j.id == data_submission_job.id && j.status == JobStatus::Created));
    assert!(jobs_response.iter().any(|j| j.id == state_transition_job_updated.id && j.status == JobStatus::Completed));
}

#[tokio::test]
#[rstest]
async fn test_get_job_status_by_block_number_not_found(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let block_number = 404;

    // Create a job for a different block to ensure db is not empty
    let other_block_job = build_job_item(JobType::SnosRun, JobStatus::Completed, block_number + 10);
    config.database().create_job(other_block_job).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .uri(format!("http://{}/jobs/block/{}/status", addr, block_number))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200); // Endpoint itself is found, just no data for this block
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<crate::server::types::JobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(response_body.message, Some(format!("Successfully fetched job statuses for block {}", block_number)));
    let jobs_response = response_body.data.unwrap().jobs;
    assert_eq!(jobs_response.len(), 0);
}
