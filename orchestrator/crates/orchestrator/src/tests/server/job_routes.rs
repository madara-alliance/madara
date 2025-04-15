use core::panic;
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

use crate::config::Config;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::types::{JobStatus, JobType};
use crate::jobs::{Job, MockJob};
use crate::queue::init_consumers;
use crate::queue::job_queue::{JobQueueMessage, QueueNameForJobType};
use crate::routes::types::ApiResponse;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::build_job_item;

#[fixture]
async fn setup_trigger() -> (SocketAddr, Arc<Config>) {
    dotenvy::from_filename("../.env.test").expect("Failed to load the .env.test file");

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
async fn test_trigger_process_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), JobStatus::Created, 1);
    config.database().create_job_item(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

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

    // Verify job was added to process queue
    let queue_message = config.queue().consume_message_from_queue(job_type.process_queue_name()).await.unwrap();
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
async fn test_trigger_verify_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    // Create a job with initial metadata
    let mut job_item = build_job_item(job_type.clone(), JobStatus::PendingVerification, 1);

    // Set verification counters in common metadata
    job_item.metadata.common.verification_retry_attempt_no = 0;
    job_item.metadata.common.verification_attempt_no = 10;

    config.database().create_job_item(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    // Set up mock job handler
    let mut job_handler = MockJob::new();
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));

    let ctx = mock_factory::get_job_handler_context();
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

    // Verify job was added to verification queue
    let queue_message = config.queue().consume_message_from_queue(job_type.verify_queue_name()).await.unwrap();
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
    config.database().create_job_item(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

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

    // Verify job was added to process queue
    let queue_message = config.queue().consume_message_from_queue(job_type.process_queue_name()).await.unwrap();

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
    config.database().create_job_item(job_item.clone()).await.unwrap();
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

#[rstest]
#[tokio::test]
async fn test_init_consumer() {
    let services = TestConfigBuilder::new().build().await;
    assert!(init_consumers(services.config).await.is_ok());
}
