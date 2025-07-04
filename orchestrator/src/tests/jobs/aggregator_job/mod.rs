use super::super::common::default_job_item;
use crate::core::client::database::MockDatabaseClient;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::batch::{Batch, BatchStatus};
use crate::types::constant::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, PROOF_FILE_NAME, STORAGE_ARTIFACTS_DIR};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::aggregator::AggregatorJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use httpmock::prelude::*;
use mockall::predicate::eq;
use orchestrator_prover_client_interface::{AtlanticStatusType, MockProverClient, TaskStatus, TaskType};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use url::Url;
use uuid::Uuid;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    // The following happens in the aggregator job creation:
    // 1. Create a job item and return it

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Aggregator(AggregatorMetadata::default()),
    };

    let job = AggregatorJobHandler.create_job(String::from("0"), metadata).await;
    assert!(job.is_ok());

    let job = job.unwrap();

    let job_type = job.job_type;
    assert_eq!(job_type, JobType::Aggregator, "job_type should be Aggregator");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn test_verify_job(#[from(default_job_item)] mut job_item: JobItem) {
    // The following things happen in the aggregator job verification:
    // 1. Get the task status from the prover client
    // 2. If the task is completed, get the artifacts from the prover client
    // 3. Calculate the program output from cairo pie
    // 4. Store the artifacts in the storage client
    // 5. Update the job status in the database

    // Fetching the aggregator cairo pie
    let mut file =
        File::open(Path::new(&format!("{}/src/tests/artifacts/aggregator.zip", env!("CARGO_MANIFEST_DIR")))).unwrap();
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).unwrap();
    let buffer_bytes = Bytes::from(buffer);

    // Mocking prover client
    let mut prover_client = MockProverClient::new();
    prover_client
        .expect_get_task_status()
        .with(eq(AtlanticStatusType::Bucket), eq("bucket_id".to_string()), eq(None), eq(false))
        .times(1)
        .returning(|_, _, _, _| Ok(TaskStatus::Succeeded)); // Testing for the case when the task is completed
    prover_client.expect_get_task_artifacts().times(2).returning(move |_, _, file_name| {
        println!("file_name: {}", file_name);
        if file_name == "pie.cairo0.zip" {
            println!("returning file_content for cairo pie zip");
            // return the actual cairo pie so we can calculate the program output
            Ok(buffer_bytes.to_vec())
        } else {
            Ok("file_content".to_string().into_bytes().to_vec())
        }
    });

    // Mocking database client
    let mut database_client = MockDatabaseClient::new();
    database_client
        .expect_update_batch_status_by_index()
        .with(eq(1), eq(BatchStatus::ReadyForStateUpdate))
        .returning(|_, _| {
            Ok(Batch { id: Uuid::default(), status: BatchStatus::ReadyForStateUpdate, ..Default::default() })
        })
        .times(1);

    let services = TestConfigBuilder::new()
        .configure_prover_client(prover_client.into())
        .configure_database(database_client.into())
        .configure_storage_client(ConfigType::Actual)
        .build()
        .await;

    job_item.metadata.specific = JobSpecificMetadata::Aggregator(AggregatorMetadata {
        batch_num: 1,
        bucket_id: "bucket_id".to_string(),
        download_proof: Some(format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, 1, PROOF_FILE_NAME)),
        cairo_pie_path: format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, 1, CAIRO_PIE_FILE_NAME),
        program_output_path: format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, 1, PROGRAM_OUTPUT_FILE_NAME),
        ..Default::default()
    });

    assert!(AggregatorJobHandler.verify_job(services.config, &mut job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    // The following things happen in the aggregator job processing:
    // 1. Submit a task to the prover client
    // 2. Update the batch status in the database

    // Mocking prover client
    let server = MockServer::start();
    let mut prover_client = MockProverClient::new();
    prover_client.expect_submit_task().times(1).returning(|_| Ok("bucket_id".to_string()));

    // Mocking database client
    let mut database_client = MockDatabaseClient::new();
    database_client
        .expect_update_batch_status_by_index()
        .with(eq(1), eq(BatchStatus::PendingAggregatorVerification))
        .returning(|_, _| {
            Ok(Batch { id: Uuid::default(), status: BatchStatus::PendingAggregatorVerification, ..Default::default() })
        })
        .times(1);

    // Creating a dummy provider
    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_prover_client(prover_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Aggregator(AggregatorMetadata {
            bucket_id: "bucket_id".to_string(),
            batch_num: 1,
            ..Default::default()
        }),
    };

    assert_eq!(
        AggregatorJobHandler
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
        "bucket_id".to_string()
    );
}
