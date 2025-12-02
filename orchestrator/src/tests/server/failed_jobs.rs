use std::net::SocketAddr;
use std::sync::Arc;

use hyper::{Body, Request};
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
async fn test_get_failed_jobs(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;

    // Create a failed job
    let failed_job = build_job_item(JobType::SnosRun, JobStatus::Failed, 1);
    config.database().create_job(failed_job.clone()).await.unwrap();

    // Create a successful job (should not be returned)
    let success_job = build_job_item(JobType::ProofCreation, JobStatus::Completed, 2);
    config.database().create_job(success_job.clone()).await.unwrap();

    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/jobs?status=failed", addr)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let body_bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
    let response_body: ApiResponse<crate::server::types::JobStatusResponse> =
        serde_json::from_slice(&body_bytes).unwrap();

    assert!(response_body.success);
    let failed_jobs_data = response_body.data.unwrap();

    let jobs_response = failed_jobs_data.jobs;

    // Verify we have at least one failed job
    assert!(!jobs_response.is_empty(), "Should have at least one failed job");
    assert!(jobs_response.len() == 1, "Should have exactly one failed job");
    // Filter to find our specific job, as DB might have other jobs from other tests if not cleaned
    let found_failed_job = jobs_response.iter().find(|j| j.id == failed_job.id);
    assert!(found_failed_job.is_some(), "Failed job should be in the response");
    let found_failed_job = found_failed_job.unwrap();
    assert_eq!(found_failed_job.job_type, JobType::SnosRun);
    assert_eq!(found_failed_job.status, JobStatus::Failed);

    let found_success_job = jobs_response.iter().find(|j| j.id == success_job.id);
    assert!(found_success_job.is_none(), "Success job should NOT be in the response");
}
