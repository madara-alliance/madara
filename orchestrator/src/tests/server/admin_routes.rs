use httpmock::MockServer;
use mockall::predicate::eq;
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::server::route::admin::BulkJobResponse;
use crate::server::types::ApiResponse;
use crate::tests::config::TestConfigBuilder;
use crate::tests::utils::build_job_item;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::queue::QueueType;
use hyper::{Body, Request};
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;

/// Test that POST /admin/jobs/retry/failed retries all failed jobs
#[tokio::test]
#[rstest]
async fn test_admin_retry_all_failed_jobs() {
    dotenvy::from_filename_override("../.env.test").ok();
    // Set required env var if not present (for mocked tests)
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }

    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let da_client = MockDaClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let job_type = JobType::DataSubmission;

    // Create failed jobs
    let failed_job_1 = build_job_item(job_type.clone(), JobStatus::Failed, 1001);
    let failed_job_2 = build_job_item(job_type.clone(), JobStatus::Failed, 1002);
    let job_id_1 = failed_job_1.id;
    let job_id_2 = failed_job_2.id;

    let failed_jobs = vec![failed_job_1.clone(), failed_job_2.clone()];

    // Mock get_jobs_by_types_and_statuses to return failed jobs
    db.expect_get_jobs_by_types_and_statuses()
        .withf(|types, statuses, _| types.is_empty() && statuses == &vec![JobStatus::Failed])
        .times(1)
        .returning(move |_, _, _| Ok(failed_jobs.clone()));

    // Mock get_job_by_id for each retry
    let job1_clone = failed_job_1.clone();
    db.expect_get_job_by_id().with(eq(job_id_1)).times(1).returning(move |_| Ok(Some(job1_clone.clone())));

    let job2_clone = failed_job_2.clone();
    db.expect_get_job_by_id().with(eq(job_id_2)).times(1).returning(move |_| Ok(Some(job2_clone.clone())));

    // Mock update_job for transitioning to PendingRetry
    db.expect_update_job().times(2).returning(move |job, _| Ok(job.clone()));

    // Mock queue send_message for adding jobs to process queue
    queue
        .expect_send_message()
        .times(2)
        .withf(|queue_type, _, _| *queue_type == QueueType::DataSubmissionJobProcessing)
        .returning(|_, _, _| Ok(()));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .configure_api_server(crate::tests::config::ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/admin/jobs/retry/failed", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse<BulkJobResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);

    let data = response.data.unwrap();
    assert_eq!(data.success_count, 2);
    assert_eq!(data.failed_count, 0);
}

/// Test that POST /admin/jobs/requeue/pending-verification requeues pending verification jobs
#[tokio::test]
#[rstest]
async fn test_admin_requeue_pending_verification() {
    dotenvy::from_filename_override("../.env.test").ok();
    // Set required env var if not present (for mocked tests)
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }

    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let da_client = MockDaClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let job_type = JobType::DataSubmission;

    // Create pending verification job
    let pending_job = build_job_item(job_type.clone(), JobStatus::PendingVerification, 2001);
    let pending_jobs = vec![pending_job.clone()];

    // Mock get_jobs_by_types_and_statuses
    db.expect_get_jobs_by_types_and_statuses()
        .withf(|types, statuses, _| types.is_empty() && statuses == &vec![JobStatus::PendingVerification])
        .times(1)
        .returning(move |_, _, _| Ok(pending_jobs.clone()));

    // Mock queue send_message for adding job to verification queue
    queue
        .expect_send_message()
        .times(1)
        .withf(|queue_type, _, _| *queue_type == QueueType::DataSubmissionJobVerification)
        .returning(|_, _, _| Ok(()));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .configure_api_server(crate::tests::config::ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/admin/jobs/requeue/pending-verification", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse<BulkJobResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);

    let data = response.data.unwrap();
    assert_eq!(data.success_count, 1);
    assert_eq!(data.failed_count, 0);
}

/// Test that POST /admin/jobs/retry/verification-timeout retries verification-timeout jobs
#[tokio::test]
#[rstest]
async fn test_admin_retry_verification_timeout_jobs() {
    dotenvy::from_filename_override("../.env.test").ok();
    // Set required env var if not present (for mocked tests)
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }

    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let da_client = MockDaClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let job_type = JobType::DataSubmission;

    // Create verification timeout job
    let timeout_job = build_job_item(job_type.clone(), JobStatus::VerificationTimeout, 4001);
    let job_id = timeout_job.id;

    let timeout_jobs = vec![timeout_job.clone()];

    // Mock get_jobs_by_types_and_statuses to return verification timeout jobs
    db.expect_get_jobs_by_types_and_statuses()
        .withf(|types, statuses, _| types.is_empty() && statuses == &vec![JobStatus::VerificationTimeout])
        .times(1)
        .returning(move |_, _, _| Ok(timeout_jobs.clone()));

    // Mock get_job_by_id for retry
    let job_clone = timeout_job.clone();
    db.expect_get_job_by_id().with(eq(job_id)).times(1).returning(move |_| Ok(Some(job_clone.clone())));

    // Mock update_job for transitioning to PendingRetry
    db.expect_update_job().times(1).returning(move |job, _| Ok(job.clone()));

    // Mock queue send_message for adding job to process queue
    queue
        .expect_send_message()
        .times(1)
        .withf(|queue_type, _, _| *queue_type == QueueType::DataSubmissionJobProcessing)
        .returning(|_, _, _| Ok(()));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .configure_api_server(crate::tests::config::ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/admin/jobs/retry/verification-timeout", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse<BulkJobResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);

    let data = response.data.unwrap();
    assert_eq!(data.success_count, 1);
    assert_eq!(data.failed_count, 0);
}

/// Test that POST /admin/jobs/requeue/created requeues created jobs
#[tokio::test]
#[rstest]
async fn test_admin_requeue_created_jobs() {
    dotenvy::from_filename_override("../.env.test").ok();
    // Set required env var if not present (for mocked tests)
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }

    let server = MockServer::start();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();
    let da_client = MockDaClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let job_type = JobType::DataSubmission;

    // Create job in Created status
    let created_job = build_job_item(job_type.clone(), JobStatus::Created, 3001);
    let created_jobs = vec![created_job.clone()];

    // Mock get_jobs_by_types_and_statuses
    db.expect_get_jobs_by_types_and_statuses()
        .withf(|types, statuses, _| types.is_empty() && statuses == &vec![JobStatus::Created])
        .times(1)
        .returning(move |_, _, _| Ok(created_jobs.clone()));

    // Mock queue send_message for adding job to process queue
    queue
        .expect_send_message()
        .times(1)
        .withf(|queue_type, _, _| *queue_type == QueueType::DataSubmissionJobProcessing)
        .returning(|_, _, _| Ok(()));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .configure_api_server(crate::tests::config::ConfigType::Actual)
        .build()
        .await;

    let addr = services.api_server_address.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{}/admin/jobs/requeue/created", addr))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response: ApiResponse<BulkJobResponse> = serde_json::from_slice(&body_bytes).unwrap();
    assert!(response.success);

    let data = response.data.unwrap();
    assert_eq!(data.success_count, 1);
    assert_eq!(data.failed_count, 0);
}
