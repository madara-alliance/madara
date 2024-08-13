use crate::config::{config, Config};
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::tests::config::TestConfigBuilder;
use arc_swap::Guard;
use rstest::*;
use std::sync::Arc;
use uuid::Uuid;

#[rstest]
#[tokio::test]
async fn test_database_connection() -> color_eyre::Result<()> {
    TestConfigBuilder::new().build().await;
    Ok(())
}

#[fixture]
async fn get_config() -> Guard<Arc<Config>> {
    config().await
}

/// Tests for `create_job` operation in database trait.
/// Creates 3 jobs and asserts them.
#[rstest]
#[tokio::test]
async fn test_database_create_job(#[future] get_config: Guard<Arc<Config>>) -> color_eyre::Result<()> {
    TestConfigBuilder::new().build().await;
    let config = get_config.await;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::ProofCreation, JobStatus::Created, 1),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();

    let get_job_1 =
        database_client.get_job_by_internal_id_and_type("1", &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_2 =
        database_client.get_job_by_internal_id_and_type("2", &JobType::ProofCreation).await.unwrap().unwrap();
    let get_job_3 =
        database_client.get_job_by_internal_id_and_type("3", &JobType::ProofCreation).await.unwrap().unwrap();

    assert_eq!(get_job_1, job_vec[0].clone());
    assert_eq!(get_job_2, job_vec[1].clone());
    assert_eq!(get_job_3, job_vec[2].clone());

    Ok(())
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
    }
}
