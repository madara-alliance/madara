use std::fs::File;
use std::io::Read;
use std::path::Path;

use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use httpmock::prelude::*;
use mockall::predicate::{always, eq};
use orchestrator_prover_client_interface::{MockProverClient, TaskStatus};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;
use uuid::Uuid;

use super::super::common::default_job_item;
use crate::core::client::database::MockDatabaseClient;
use crate::core::client::storage::MockStorageClient;
use crate::core::config::ProverKind;
use crate::tests::config::TestConfigBuilder;
use crate::types::constant::CAIRO_PIE_FILE_NAME;
use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::proving::proving_job_tracking_id;
use crate::worker::event_handler::jobs::proving::ProvingJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata::default()),
    };

    let job = ProvingJobHandler.create_job(0, metadata).await;
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

    prover_client.expect_submit_task().with(always()).times(1).returning(|task| match task {
        orchestrator_prover_client_interface::Task::CreateJob(info) => {
            assert_eq!(info.dedup_id, "00000000-0000-0000-0000-000000000000");
            Ok("task_id".to_string())
        }
        other => panic!("unexpected task submitted: {:?}", std::mem::discriminant(&other)),
    });
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
        .configure_prover_kind(ProverKind::Atlantic)
        .configure_prover_client(prover_client.into())
        .configure_storage_client(storage.into())
        .build()
        .await;

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputType::CairoPie(cairo_pie_path)),
            ..Default::default()
        }),
    };

    assert_eq!(
        ProvingJobHandler
            .process_job(
                services.config,
                &mut JobItem {
                    id: Uuid::default(),
                    internal_id: 0,
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

#[rstest]
#[case(ProverKind::Sharp, "0xsharpfeedface")]
#[case(ProverKind::Mock, "0xmockfeedface")]
#[tokio::test]
async fn test_process_job_uses_snos_fact_for_non_atlantic_provers(
    #[case] prover_kind: ProverKind,
    #[case] snos_fact: &str,
) {
    let server = MockServer::start();
    let mut prover_client = MockProverClient::new();
    let mut database = MockDatabaseClient::new();
    let expected_dedup_id = snos_fact.to_string();
    let snos_fact_for_db = expected_dedup_id.clone();

    prover_client.expect_submit_task().with(always()).times(1).returning(|task| match task {
        orchestrator_prover_client_interface::Task::CreateJob(info) => Ok(info.dedup_id),
        other => panic!("unexpected task submitted: {:?}", std::mem::discriminant(&other)),
    });

    database.expect_get_job_by_internal_id_and_type().with(eq(0_u64), eq(JobType::SnosRun)).times(1).returning(
        move |_, _| {
            Ok(Some(JobItem {
                id: Uuid::new_v4(),
                internal_id: 0,
                job_type: JobType::SnosRun,
                status: JobStatus::Completed,
                external_id: ExternalId::String("0".to_string().into_boxed_str()),
                metadata: JobMetadata {
                    common: CommonMetadata::default(),
                    specific: JobSpecificMetadata::Snos(SnosMetadata {
                        start_block: 0,
                        end_block: 0,
                        num_blocks: 1,
                        full_output: true,
                        snos_fact: Some(snos_fact_for_db.clone()),
                        ..Default::default()
                    }),
                },
                version: 0,
                created_at: Utc::now().round_subsecs(0),
                updated_at: Utc::now().round_subsecs(0),
            }))
        },
    );

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
        .configure_prover_kind(prover_kind)
        .configure_prover_client(prover_client.into())
        .configure_database(database.into())
        .configure_storage_client(storage.into())
        .build()
        .await;

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Proving(ProvingMetadata {
            input_path: Some(ProvingInputType::CairoPie(cairo_pie_path)),
            ..Default::default()
        }),
    };

    assert_eq!(
        ProvingJobHandler
            .process_job(
                services.config,
                &mut JobItem {
                    id: Uuid::default(),
                    internal_id: 0,
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
        expected_dedup_id
    );
}

#[test]
fn test_tracking_id_uses_job_uuid_for_atlantic() {
    let job_id = Uuid::nil();
    let tracking_id = proving_job_tracking_id(ProverKind::Atlantic, job_id, None).unwrap();
    assert_eq!(tracking_id, job_id.to_string());
}

#[test]
fn test_tracking_id_uses_fact_hash_for_sharp_and_mock() {
    let sharp_tracking_id =
        proving_job_tracking_id(ProverKind::Sharp, Uuid::nil(), Some("0xfeedface".to_string())).unwrap();
    let mock_tracking_id =
        proving_job_tracking_id(ProverKind::Mock, Uuid::nil(), Some("0xfeedface".to_string())).unwrap();

    assert_eq!(sharp_tracking_id, "0xfeedface");
    assert_eq!(mock_tracking_id, "0xfeedface");
}

#[test]
fn test_tracking_id_requires_snos_fact_for_sharp_and_mock() {
    assert!(proving_job_tracking_id(ProverKind::Sharp, Uuid::nil(), None).is_err());
    assert!(proving_job_tracking_id(ProverKind::Mock, Uuid::nil(), None).is_err());
}
