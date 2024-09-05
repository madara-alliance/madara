use bytes::Bytes;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use crate::config::config;
use crate::data_storage::MockDataStorage;
use httpmock::prelude::*;
use mockall::predicate::eq;
use prover_client_interface::{MockProverClient, TaskStatus};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use uuid::Uuid;

use super::super::common::default_job_item;
use crate::jobs::proving_job::ProvingJob;
use crate::jobs::types::{JobItem, JobStatus, JobType};
use crate::jobs::Job;
use crate::tests::config::TestConfigBuilder;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    TestConfigBuilder::new().build().await;
    let config = config().await;

    let job = ProvingJob.create_job(&config, String::from("0"), HashMap::new()).await;
    assert!(job.is_ok());

    let job = job.unwrap();

    let job_type = job.job_type;
    assert_eq!(job_type, JobType::ProofCreation, "job_type should be ProofCreation");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn test_verify_job(#[from(default_job_item)] mut job_item: JobItem) {
    let mut prover_client = MockProverClient::new();
    prover_client.expect_get_task_status().times(1).returning(|_| Ok(TaskStatus::Succeeded));

    TestConfigBuilder::new().mock_prover_client(Box::new(prover_client)).build().await;

    let config = config().await;
    assert!(ProvingJob.verify_job(&config, &mut job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();
    let mut prover_client = MockProverClient::new();

    prover_client.expect_submit_task().times(1).returning(|_| Ok("task_id".to_string()));
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut file =
        File::open(Path::new(&format!("{}/src/tests/artifacts/fibonacci.zip", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let mut storage = MockDataStorage::new();
    let buffer_bytes = Bytes::from(buffer);
    storage.expect_get_data().with(eq("0/pie.zip")).return_once(move |_| Ok(buffer_bytes));

    TestConfigBuilder::new()
        .mock_starknet_client(Arc::new(provider))
        .mock_prover_client(Box::new(prover_client))
        .mock_storage_client(Box::new(storage))
        .build()
        .await;

    assert_eq!(
        ProvingJob
            .process_job(
                config().await.as_ref(),
                &mut JobItem {
                    id: Uuid::default(),
                    internal_id: "0".into(),
                    job_type: JobType::ProofCreation,
                    status: JobStatus::Created,
                    external_id: String::new().into(),
                    metadata: HashMap::new(),
                    version: 0,
                }
            )
            .await
            .unwrap(),
        "task_id".to_string()
    );
}
