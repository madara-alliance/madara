use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use httpmock::prelude::*;
use mockall::predicate::eq;
use prover_client_interface::{MockProverClient, TaskStatus};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use uuid::Uuid;

use super::super::common::default_job_item;
use crate::constants::CAIRO_PIE_FILE_NAME;
use crate::data_storage::MockDataStorage;
use crate::jobs::constants::JOB_METADATA_SNOS_FACT;
use crate::jobs::proving_job::ProvingJob;
use crate::jobs::types::{JobItem, JobStatus, JobType};
use crate::jobs::Job;
use crate::tests::config::TestConfigBuilder;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let services = TestConfigBuilder::new().build().await;

    let job = ProvingJob.create_job(services.config.clone(), String::from("0"), HashMap::new()).await;
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
    prover_client.expect_get_task_status().times(1).returning(|_, _| Ok(TaskStatus::Succeeded));

    let services = TestConfigBuilder::new().configure_prover_client(prover_client.into()).build().await;

    job_item.metadata.insert(JOB_METADATA_SNOS_FACT.into(), "fact".to_string());
    assert!(ProvingJob.verify_job(services.config, &mut job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();
    let mut prover_client = MockProverClient::new();

    prover_client.expect_submit_task().times(1).returning(|_, _| Ok("task_id".to_string()));
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut file =
        File::open(Path::new(&format!("{}/src/tests/artifacts/fibonacci.zip", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let mut storage = MockDataStorage::new();
    let buffer_bytes = Bytes::from(buffer);
    storage
        .expect_get_data()
        .with(eq(format!("{}/{}", "0", CAIRO_PIE_FILE_NAME)))
        .return_once(move |_| Ok(buffer_bytes));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_prover_client(prover_client.into())
        .configure_storage_client(storage.into())
        .build()
        .await;

    assert_eq!(
        ProvingJob
            .process_job(
                services.config,
                &mut JobItem {
                    id: Uuid::default(),
                    internal_id: "0".into(),
                    job_type: JobType::ProofCreation,
                    status: JobStatus::Created,
                    external_id: String::new().into(),
                    metadata: HashMap::new(),
                    version: 0,
                    created_at: Utc::now().round_subsecs(0),
                    updated_at: Utc::now().round_subsecs(0)
                }
            )
            .await
            .unwrap(),
        "task_id".to_string()
    );
}
