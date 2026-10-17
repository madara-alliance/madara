mod local_agg_e2e;

use super::super::common::default_job_item;
use crate::core::client::database::MockDatabaseClient;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus};
use crate::types::constant::{get_batch_artifact_file, PROOF_FILE_NAME};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{AggregatorMetadata, CommonMetadata, JobMetadata, JobSpecificMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::aggregator::AggregatorJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use mockall::predicate::eq;
use orchestrator_prover_client_interface::{MockProverClient, TaskStatus, TaskType};
use rstest::*;
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

    let job = AggregatorJobHandler.create_job(0, metadata).await;
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
    // 2. If the task is completed, get the optional proof from the prover client
    // 3. Update the job status in the database

    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    // Mocking prover client
    let mut prover_client = MockProverClient::new();
    prover_client
        .expect_get_task_status()
        .with(eq(TaskType::Aggregation), eq("aggregation_id".to_string()), eq(None))
        .times(1)
        .returning(|_, _, _| Ok(TaskStatus::Succeeded));
    prover_client.expect_get_aggregation_artifacts().times(1).returning(|_, _| {
        Ok(orchestrator_prover_client_interface::AggregationArtifacts { proof: Some("proof".to_string().into_bytes()) })
    });

    // Mocking database client
    let mut database_client = MockDatabaseClient::new();
    database_client
        .expect_update_aggregator_batch_status_by_index()
        .with(eq(1), eq(AggregatorBatchStatus::ReadyForStateUpdate))
        .returning(|_, _| {
            Ok(AggregatorBatch {
                id: Uuid::default(),
                status: AggregatorBatchStatus::ReadyForStateUpdate,
                ..Default::default()
            })
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
        download_proof: Some(get_batch_artifact_file(1, PROOF_FILE_NAME)),
        ..Default::default()
    });
    job_item.external_id = "aggregation_id".to_string().into();

    assert!(AggregatorJobHandler.verify_job(services.config, &mut job_item).await.is_ok());
}
