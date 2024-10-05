use chrono::{SubsecRound, Utc};
use rstest::*;
use uuid::Uuid;

use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::tests::config::{ConfigType, TestConfigBuilder};

#[rstest]
#[tokio::test]
async fn test_database_connection() -> color_eyre::Result<()> {
    let _services = TestConfigBuilder::new().build().await;
    Ok(())
}

/// Tests for `create_job` operation in database trait.
/// Creates 3 jobs and asserts them.
#[rstest]
#[tokio::test]
async fn database_create_job_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
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
}

/// Test for `get_jobs_without_successor` operation in database trait.
/// Creates jobs in the following sequence :
///
/// - Creates 3 snos run jobs with completed status
///
/// - Creates 2 proof creation jobs with succession of the 2 snos jobs
///
/// - Should return one snos job without the successor job of proof creation
#[rstest]
#[case(true)]
#[case(false)]
#[tokio::test]
async fn database_get_jobs_without_successor_works(#[case] is_successor: bool) {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 1),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Created, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();
    if is_successor {
        database_client.create_job(job_vec[4].clone()).await.unwrap();
    }

    let jobs_without_successor = database_client
        .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
        .await
        .unwrap();

    if is_successor {
        assert_eq!(jobs_without_successor.len(), 0, "Expected number of jobs assertion failed.");
    } else {
        assert_eq!(jobs_without_successor.len(), 1, "Expected number of jobs assertion failed.");
        assert_eq!(jobs_without_successor[0], job_vec[1], "Expected job assertion failed.");
    }
}

/// Test for `get_latest_job_by_type` operation in database trait.
/// Creates the jobs in following sequence :
///
/// - Creates 3 successful jobs.
///
/// - Should return the last successful job
#[rstest]
#[tokio::test]
async fn database_get_last_successful_job_by_type_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 3),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();

    let last_successful_job = database_client.get_latest_job_by_type(JobType::SnosRun).await.unwrap().unwrap();

    assert_eq!(last_successful_job, job_vec[2], "Expected job assertion failed");
}

/// Test for `get_jobs_after_internal_id_by_job_type` operation in database trait.
/// Creates the jobs in following sequence :
///
/// - Creates 5 successful jobs.
///
/// - Should return the jobs after internal id
#[rstest]
#[tokio::test]
async fn database_get_jobs_after_internal_id_by_job_type_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    let job_vec = [
        build_job_item(JobType::SnosRun, JobStatus::Completed, 1),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 2),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 3),
        build_job_item(JobType::ProofCreation, JobStatus::Completed, 4),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 5),
        build_job_item(JobType::SnosRun, JobStatus::Completed, 6),
    ];

    database_client.create_job(job_vec[0].clone()).await.unwrap();
    database_client.create_job(job_vec[1].clone()).await.unwrap();
    database_client.create_job(job_vec[2].clone()).await.unwrap();
    database_client.create_job(job_vec[3].clone()).await.unwrap();
    database_client.create_job(job_vec[4].clone()).await.unwrap();
    database_client.create_job(job_vec[5].clone()).await.unwrap();

    let jobs_after_internal_id = database_client
        .get_jobs_after_internal_id_by_job_type(JobType::SnosRun, JobStatus::Completed, "2".to_string())
        .await
        .unwrap();

    assert_eq!(jobs_after_internal_id.len(), 2, "Number of jobs assertion failed");
    assert_eq!(jobs_after_internal_id[0], job_vec[4]);
    assert_eq!(jobs_after_internal_id[1], job_vec[5]);
}

/// Test for `update_job_status` operation in database trait.
/// Happy Case : Creating a job with version 0 and updating the job with version 0 update only.
#[rstest]
#[tokio::test]
async fn database_update_job_status_passing_case_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;

    let database_client = config.database();

    let job = build_job_item(JobType::SnosRun, JobStatus::Created, 1);

    database_client.create_job(job.clone()).await.unwrap();

    let updating_job_res = database_client.update_job_status(&job, JobStatus::Completed).await.is_ok();

    assert!(updating_job_res, "Job result assertion failed");
}

/// Test for `update_job_status` operation in database trait.
/// Failing Case : Creating a job with version 1 and updating the job with version 0 update only.
#[rstest]
#[tokio::test]
async fn database_update_job_status_failing_case_works() {
    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;
    let config = services.config;
    let database_client = config.database();

    // Scenario :

    // Worker 1 :
    // Job is created
    let job = build_job_item(JobType::SnosRun, JobStatus::Created, 1);
    database_client.create_job(job.clone()).await.unwrap();

    // Worker 2 :
    // Job is updated
    let updating_job_res = database_client.update_job_status(&job, JobStatus::Completed).await.is_ok();
    assert!(updating_job_res, "Job result assertion failed");

    // Worker 1 :
    // Job update try (update should fail)
    let updating_job_res = database_client.update_job_status(&job, JobStatus::PendingVerification).await.is_err();
    assert!(updating_job_res, "Job result assertion failed");
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
