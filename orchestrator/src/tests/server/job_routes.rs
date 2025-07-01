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
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::utils::build_job_item;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueNameForJobType;
use crate::worker::event_handler::factory::mock_factory::get_job_handler_context;
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
async fn test_trigger_process_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;
    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), JobStatus::Created, 1);
    config.database().create_job(job_item.clone()).await.unwrap();
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

    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    // Set up mock job handler
    let mut job_handler = MockJobHandlerTrait::new();
    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);
    let job_handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(job_handler));

    let ctx = get_job_handler_context();
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
    config.database().create_job(job_item.clone()).await.unwrap();
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
    let snos_job = build_job_item(JobType::SnosRun, JobStatus::Completed, block_number);
    let proving_job = build_job_item(JobType::ProofCreation, JobStatus::PendingVerification, block_number);
    let data_submission_job = build_job_item(JobType::DataSubmission, JobStatus::Created, block_number);
    let state_transition_job =
        build_job_item(JobType::StateTransition, JobStatus::Completed, 0); // internal_id is not block_number for ST
    let mut state_transition_job_specific_metadata =
        state_transition_job.metadata.specific.clone().try_into_state_transition().unwrap();
    state_transition_job_specific_metadata.blocks_to_settle = vec![block_number, block_number + 1];
    let mut state_transition_job_updated = state_transition_job.clone();
    state_transition_job_updated.metadata.specific = state_transition_job_specific_metadata.into();

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
    let response_body: ApiResponse<crate::server::types::BlockJobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(
        response_body.message,
        Some(format!("Successfully fetched job statuses for block {}", block_number))
    );
    let jobs_response = response_body.data.unwrap().jobs;
    assert_eq!(jobs_response.len(), 4);

    // Check that the correct jobs are returned
    assert!(jobs_response.iter().any(|j| j.id == snos_job.id && j.status == JobStatus::Completed));
    assert!(jobs_response.iter().any(|j| j.id == proving_job.id && j.status == JobStatus::PendingVerification));
    assert!(jobs_response.iter().any(|j| j.id == data_submission_job.id && j.status == JobStatus::Created));
    assert!(
        jobs_response.iter().any(|j| j.id == state_transition_job_updated.id && j.status == JobStatus::Completed)
    );
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
    let response_body: ApiResponse<crate::server::types::BlockJobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    assert_eq!(
        response_body.message,
        Some(format!("Successfully fetched job statuses for block {}", block_number))
    );
    let jobs_response = response_body.data.unwrap().jobs;
    assert_eq!(jobs_response.len(), 0);
}

#[tokio::test]
#[rstest]
async fn test_get_job_status_by_block_number_l3_proof_registration(
    #[future] setup_trigger: (SocketAddr, Arc<Config>),
) {
    let (addr, mut config_arc) = setup_trigger.await;
    let block_number = 789;

    // Modify config to be L3
    let mut config_writable = Arc::make_mut(&mut config_arc);
    config_writable.layer_config =
        Some(crate::core::config::LayerConfig { layer_type: crate::core::config::LayerType::L3, ..Default::default() });

    let proof_reg_job_l3 = build_job_item(JobType::ProofRegistration, JobStatus::Completed, block_number);
    config_arc.database().create_job(proof_reg_job_l3.clone()).await.unwrap();

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
    let response_body: ApiResponse<crate::server::types::BlockJobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();
    assert!(response_body.success);
    let jobs_response = response_body.data.unwrap().jobs;
    assert_eq!(jobs_response.len(), 1);
    assert_eq!(jobs_response[0].id, proof_reg_job_l3.id);

    // Modify config to be L2
    let mut config_writable_l2 = Arc::make_mut(&mut config_arc);
    config_writable_l2.layer_config =
        Some(crate::core::config::LayerConfig { layer_type: crate::core::config::LayerType::L2, ..Default::default() });

    // ProofRegistration job for the same block, but now config is L2
    // We don't need to re-insert the job, just re-query with the updated config context (if State was properly passed)
    // However, the config is passed as Arc, so the handler will use the config state at the time of the request.
    // To test this properly, we would ideally need to restart the server with new config or have a mutable config.
    // For this test, we'll simulate by creating a new job and querying again, assuming the server was "restarted" with L2 config.
    // OR, rely on the handler logic to correctly use the config passed in its State.
    // The current setup_trigger provides a new config for each test, so we can create a new one.

    // Let's assume for this part of the test, we'd have a separate fixture or modify the existing one.
    // For simplicity here, we'll clear the DB and re-insert for an L2 scenario.
    // This is not ideal as it tests DB interaction more than config propagation in a single server instance.
    // A better approach would be to have specific test setups for L2 and L3 configs.

    // For now, let's test that if it *were* an L2 config, the job *would not* be returned.
    // We will rely on the handler logic using the `config` passed in `State`.
    // We create a new config that is L2.
    let madara_url = get_env_var_or_panic("MADARA_ORCHESTRATOR_MADARA_RPC_URL");
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(madara_url.as_str().to_string().as_str()).expect("Failed to parse URL"),
    ));
    let services_l2 = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual) // Use the same DB
        .configure_queue_client(ConfigType::Actual)
        .configure_starknet_client(provider.into())
        .configure_api_server(ConfigType::Actual) // New server instance effectively
        .with_layer_type(crate::core::config::LayerType::L2) // Explicitly L2
        .build()
        .await;

    // Ensure the job still exists in the DB
    let _ = services_l2.config.database().create_job(proof_reg_job_l3.clone()).await;


    let client_l2 = hyper::Client::new();
    let response_l2 = client_l2
        .request(
            Request::builder()
                .uri(format!("http://{}/jobs/block/{}/status", services_l2.api_server_address.unwrap(), block_number))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response_l2.status(), 200);
    let body_bytes_l2 = hyper::body::to_bytes(response_l2.into_body()).await.unwrap();
    let response_body_l2: ApiResponse<crate::server::types::BlockJobStatusResponse> =
        serde_json::from_slice(&body_bytes_l2).unwrap();
    assert!(response_body_l2.success);
    let jobs_response_l2 = response_body_l2.data.unwrap().jobs;
    assert_eq!(jobs_response_l2.len(), 0, "ProofRegistration job should not be returned for L2 config");
}
