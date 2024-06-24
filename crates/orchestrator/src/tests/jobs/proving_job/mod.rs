use std::collections::HashMap;

use httpmock::prelude::*;
use prover_client_interface::{MockProverClient, TaskStatus};
use rstest::*;
use uuid::Uuid;

use super::super::common::{default_job_item, init_config};
use crate::jobs::constants::JOB_METADATA_CAIRO_PIE_PATH_KEY;
use crate::jobs::proving_job::ProvingJob;
use crate::jobs::types::{JobItem, JobStatus, JobType};
use crate::jobs::Job;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let config = init_config(None, None, None, None, None).await;
    let job = ProvingJob
        .create_job(
            &config,
            String::from("0"),
            HashMap::from([(JOB_METADATA_CAIRO_PIE_PATH_KEY.into(), "pie.zip".into())]),
        )
        .await;
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
async fn test_verify_job(#[from(default_job_item)] job_item: JobItem) {
    let mut prover_client = MockProverClient::new();
    prover_client.expect_get_task_status().times(1).returning(|_| Ok(TaskStatus::Succeeded));

    let config = init_config(None, None, None, None, Some(prover_client)).await;
    assert!(ProvingJob.verify_job(&config, &job_item).await.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();

    let mut prover_client = MockProverClient::new();
    prover_client.expect_submit_task().times(1).returning(|_| Ok("task_id".to_string()));

    let config =
        init_config(Some(format!("http://localhost:{}", server.port())), None, None, None, Some(prover_client)).await;

    let cairo_pie_path = format!("{}/src/tests/artifacts/fibonacci.zip", env!("CARGO_MANIFEST_DIR"));

    assert_eq!(
        ProvingJob
            .process_job(
                &config,
                &JobItem {
                    id: Uuid::default(),
                    internal_id: "0".into(),
                    job_type: JobType::ProofCreation,
                    status: JobStatus::Created,
                    external_id: String::new().into(),
                    metadata: HashMap::from([(JOB_METADATA_CAIRO_PIE_PATH_KEY.into(), cairo_pie_path)]),
                    version: 0,
                }
            )
            .await
            .unwrap(),
        "task_id".to_string()
    );
}
