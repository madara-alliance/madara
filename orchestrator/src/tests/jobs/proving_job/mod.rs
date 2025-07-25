use std::fs::File;
use std::io::Read;
use std::path::Path;

use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use httpmock::prelude::*;
use mockall::predicate::eq;
use orchestrator_prover_client_interface::{MockProverClient, TaskStatus};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use uuid::Uuid;

use super::super::common::default_job_item;
use crate::core::client::storage::MockStorageClient;
use crate::tests::config::TestConfigBuilder;
use crate::types::constant::CAIRO_PIE_FILE_NAME;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::proving::ProvingJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata::default()),
    };

    let job = ProvingJobHandler.create_job(String::from("0"), metadata).await;
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
    prover_client.expect_get_task_status().times(1).returning(|_, _, _| Ok(TaskStatus::Succeeded));

    let services = TestConfigBuilder::new().configure_prover_client(prover_client.into()).build().await;

    job_item.metadata.specific = JobSpecificMetadata::Proving(ProvingMetadata {
        ensure_on_chain_registration: Some("fact".to_string()),
        ..Default::default()
    });

    assert!(ProvingJobHandler.verify_job(services.config, &mut job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();
    let mut prover_client = MockProverClient::new();

    prover_client.expect_submit_task().times(1).returning(|_, _, _| Ok("task_id".to_string()));
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let mut file =
        File::open(Path::new(&format!("{}/src/tests/artifacts/fibonacci.zip", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();

    let mut storage = MockStorageClient::new();
    let buffer_bytes = Bytes::from(buffer);
    let cairo_pie_path = format!("0/{}", CAIRO_PIE_FILE_NAME);
    storage.expect_get_data().with(eq(cairo_pie_path.clone())).return_once(move |_| Ok(buffer_bytes));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_prover_client(prover_client.into())
        .configure_storage_client(storage.into())
        .build()
        .await;

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputType::CairoPie(cairo_pie_path)),
            ensure_on_chain_registration: Some("fact".to_string()),
            ..Default::default()
        }),
    };

    assert_eq!(
        ProvingJobHandler
            .process_job(
                services.config,
                &mut JobItem {
                    id: Uuid::default(),
                    internal_id: "0".into(),
                    job_type: JobType::ProofCreation,
                    status: JobStatus::Created,
                    external_id: String::new().into(),
                    metadata,
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
