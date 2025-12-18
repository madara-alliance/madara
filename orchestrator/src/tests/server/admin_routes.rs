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

fn setup_env() {
    dotenvy::from_filename_override("../.env.test").ok();
    if std::env::var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL").is_err() {
        std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    }
}

async fn build_test_config(db: MockDatabaseClient, queue: MockQueueClient) -> String {
    let server = MockServer::start();
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));
    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(MockDaClient::new().into())
        .configure_prover_client(MockProverClient::new().into())
        .configure_settlement_client(MockSettlementClient::new().into())
        .configure_api_server(crate::tests::config::ConfigType::Actual)
        .build()
        .await;
    services.api_server_address.unwrap().to_string()
}

async fn call_endpoint(addr: &str, path: &str) -> ApiResponse<BulkJobResponse> {
    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().method("POST").uri(format!("http://{}{}", addr, path)).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    serde_json::from_slice(&body_bytes).unwrap()
}

#[tokio::test]
#[rstest]
async fn test_admin_retry_all_failed_jobs() {
    setup_env();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    let job1 = build_job_item(JobType::DataSubmission, JobStatus::Failed, 1001);
    let job2 = build_job_item(JobType::DataSubmission, JobStatus::Failed, 1002);
    let (id1, id2) = (job1.id, job2.id);
    let jobs = vec![job1.clone(), job2.clone()];

    db.expect_get_jobs_by_types_and_statuses()
        .withf(|t, s, _| t.is_empty() && s == &vec![JobStatus::Failed])
        .times(1)
        .returning(move |_, _, _| Ok(jobs.clone()));
    let j1 = job1.clone();
    db.expect_get_job_by_id().with(eq(id1)).times(1).returning(move |_| Ok(Some(j1.clone())));
    let j2 = job2.clone();
    db.expect_get_job_by_id().with(eq(id2)).times(1).returning(move |_| Ok(Some(j2.clone())));
    db.expect_update_job().times(2).returning(|job, _| Ok(job.clone()));
    queue.expect_send_message().times(2).returning(|_, _, _| Ok(()));

    let addr = build_test_config(db, queue).await;
    let resp = call_endpoint(&addr, "/admin/jobs/retry/failed").await;
    assert!(resp.success);
    assert_eq!(resp.data.unwrap().success_count, 2);
}

#[tokio::test]
#[rstest]
async fn test_admin_retry_verification_timeout_jobs() {
    setup_env();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    let job = build_job_item(JobType::DataSubmission, JobStatus::VerificationTimeout, 4001);
    let job_id = job.id;
    let jobs = vec![job.clone()];

    db.expect_get_jobs_by_types_and_statuses()
        .withf(|t, s, _| t.is_empty() && s == &vec![JobStatus::VerificationTimeout])
        .times(1)
        .returning(move |_, _, _| Ok(jobs.clone()));
    let jc = job.clone();
    db.expect_get_job_by_id().with(eq(job_id)).times(1).returning(move |_| Ok(Some(jc.clone())));
    db.expect_update_job().times(1).returning(|job, _| Ok(job.clone()));
    queue.expect_send_message().times(1).returning(|_, _, _| Ok(()));

    let addr = build_test_config(db, queue).await;
    let resp = call_endpoint(&addr, "/admin/jobs/retry/verification-timeout").await;
    assert!(resp.success);
    assert_eq!(resp.data.unwrap().success_count, 1);
}

#[tokio::test]
#[rstest]
async fn test_admin_requeue_pending_verification() {
    setup_env();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    let job = build_job_item(JobType::DataSubmission, JobStatus::PendingVerification, 2001);
    let jobs = vec![job.clone()];

    db.expect_get_jobs_by_types_and_statuses()
        .withf(|t, s, _| t.is_empty() && s == &vec![JobStatus::PendingVerification])
        .times(1)
        .returning(move |_, _, _| Ok(jobs.clone()));
    queue
        .expect_send_message()
        .times(1)
        .withf(|qt, _, _| *qt == QueueType::DataSubmissionJobVerification)
        .returning(|_, _, _| Ok(()));

    let addr = build_test_config(db, queue).await;
    let resp = call_endpoint(&addr, "/admin/jobs/requeue/pending-verification").await;
    assert!(resp.success);
    assert_eq!(resp.data.unwrap().success_count, 1);
}

#[tokio::test]
#[rstest]
async fn test_admin_requeue_created_jobs() {
    setup_env();
    let mut db = MockDatabaseClient::new();
    let mut queue = MockQueueClient::new();

    let job = build_job_item(JobType::DataSubmission, JobStatus::Created, 3001);
    let jobs = vec![job.clone()];

    db.expect_get_jobs_by_types_and_statuses()
        .withf(|t, s, _| t.is_empty() && s == &vec![JobStatus::Created])
        .times(1)
        .returning(move |_, _, _| Ok(jobs.clone()));
    queue
        .expect_send_message()
        .times(1)
        .withf(|qt, _, _| *qt == QueueType::DataSubmissionJobProcessing)
        .returning(|_, _, _| Ok(()));

    let addr = build_test_config(db, queue).await;
    let resp = call_endpoint(&addr, "/admin/jobs/requeue/created").await;
    assert!(resp.success);
    assert_eq!(resp.data.unwrap().success_count, 1);
}
