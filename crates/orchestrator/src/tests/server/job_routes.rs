use core::panic;
use std::net::SocketAddr;
use std::sync::Arc;

use chrono::{SubsecRound as _, Utc};
use hyper::{Body, Request};
use mockall::predicate::eq;
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use utils::env_utils::get_env_var_or_panic;
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::job_handler_factory::mock_factory;
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::{Job, MockJob};
use crate::queue::init_consumers;
use crate::tests::config::{ConfigType, TestConfigBuilder};

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
    let mut job_handler = MockJob::new();

    job_handler.expect_process_job().times(1).returning(move |_, _| Ok("0xbeef".to_string()));

    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(job_type)).returning(move |_| Arc::clone(&job_handler));

    let client = hyper::Client::new();
    let response = client
        .request(
            Request::builder().uri(format!("http://{}/jobs/{}/process", addr, job_id)).body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();

    // assertions
    if let Some(job_fetched) = config.database().get_job_by_id(job_id).await.unwrap() {
        assert_eq!(response.status(), 200);
        assert_eq!(job_fetched.id, job_item.id);
        assert_eq!(job_fetched.version, 2);
        assert_eq!(job_fetched.status, JobStatus::PendingVerification);
    } else {
        panic!("Could not get job from database")
    }
}

#[tokio::test]
#[rstest]
async fn test_trigger_verify_job(#[future] setup_trigger: (SocketAddr, Arc<Config>)) {
    let (addr, config) = setup_trigger.await;

    let job_type = JobType::DataSubmission;

    let job_item = build_job_item(job_type.clone(), JobStatus::PendingVerification, 1);
    let mut job_handler = MockJob::new();

    job_handler.expect_verify_job().times(1).returning(move |_, _| Ok(JobVerificationStatus::Verified));

    config.database().create_job(job_item.clone()).await.unwrap();
    let job_id = job_item.clone().id;

    job_handler.expect_verification_polling_delay_seconds().return_const(1u64);

    let job_handler: Arc<Box<dyn Job>> = Arc::new(Box::new(job_handler));
    let ctx = mock_factory::get_job_handler_context();
    ctx.expect().times(1).with(eq(job_type)).returning(move |_| Arc::clone(&job_handler));

    let client = hyper::Client::new();
    let response = client
        .request(Request::builder().uri(format!("http://{}/jobs/{}/verify", addr, job_id)).body(Body::empty()).unwrap())
        .await
        .unwrap();

    // assertions
    if let Some(job_fetched) = config.database().get_job_by_id(job_id).await.unwrap() {
        assert_eq!(response.status(), 200);
        assert_eq!(job_fetched.id, job_item.id);
        assert_eq!(job_fetched.version, 1);
        assert_eq!(job_fetched.status, JobStatus::Completed);
    } else {
        panic!("Could not get job from database")
    }
}

#[rstest]
#[tokio::test]
async fn test_init_consumer() {
    let services = TestConfigBuilder::new().build().await;
    assert!(init_consumers(services.config).await.is_ok());
}

// Test Util Functions
// ==========================================

pub fn build_job_item(job_type: JobType, job_status: JobStatus, internal_id: u64) -> JobItem {
    JobItem {
        id: Uuid::new_v4(),
        internal_id: internal_id.to_string(),
        job_type,
        status: job_status,
        external_id: ExternalId::Number(0),
        metadata: Default::default(),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    }
}
