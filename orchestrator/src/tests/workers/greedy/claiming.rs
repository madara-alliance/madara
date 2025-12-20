/// Tests for atomic job claiming in greedy mode
/// These tests verify that the greedy worker correctly claims jobs atomically,
/// handles concurrent claims from multiple orchestrators, and respects job priorities.
use crate::core::config::Config;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::Utc;
use rstest::rstest;
use std::sync::Arc;
use uuid::Uuid;

/// Test that a single orchestrator can successfully claim a job
#[rstest]
#[tokio::test]
async fn test_atomic_claim_single_orchestrator() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create a test job in Created status
    let job_id = Uuid::new_v4();
    job.id = job_id;
    let mut job = JobItem::create("test_internal_id_1".to_string(), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job.clone()).await.expect("Failed to create job");

    // Claim the job for processing
    let orchestrator_id = "test-orchestrator-1".to_string();
    let claimed_job = db
        .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim job")
        .expect("No job was claimed");

    // Verify the job was claimed
    assert_eq!(claimed_job.id, job_id);
    assert_eq!(claimed_job.status, JobStatus::LockedForProcessing);
    assert_eq!(claimed_job.claimed_by, Some(orchestrator_id.clone()));
    assert!(claimed_job.metadata.common.claimed_at.is_some());

    // Verify the job in database has the claim
    let db_job = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(db_job.claimed_by, Some(orchestrator_id));
    assert_eq!(db_job.status, JobStatus::LockedForProcessing);
}

/// Test that multiple orchestrators competing for the same job results in only one claim
#[rstest]
#[tokio::test]
async fn test_atomic_claim_multiple_orchestrators() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create a single job
    let job_id = Uuid::new_v4();
    job.id = job_id;
    let job = JobItem::create(
        "test_internal_id_concurrent".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job).await.expect("Failed to create job");

    // Simulate two orchestrators trying to claim the same job concurrently
    let db1 = db.clone();
    let db2 = db.clone();

    let handle1 = tokio::spawn(async move {
        db1.claim_job_for_processing(&JobType::SnosRun, "orchestrator-1").await.expect("Failed to claim")
    });

    let handle2 = tokio::spawn(async move {
        db2.claim_job_for_processing(&JobType::SnosRun, "orchestrator-2").await.expect("Failed to claim")
    });

    let (result1, result2) = tokio::join!(handle1, handle2);
    let claim1 = result1.expect("Task 1 failed");
    let claim2 = result2.expect("Task 2 failed");

    // Exactly one should succeed, one should get None
    let claims_succeeded = vec![claim1.is_some(), claim2.is_some()];
    assert_eq!(claims_succeeded.iter().filter(|&&x| x).count(), 1, "Exactly one claim should succeed");

    // Verify the job is claimed by one orchestrator
    let db_job = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(db_job.status, JobStatus::LockedForProcessing);
    assert!(db_job.claimed_by.is_some());

    let claimed_by = db_job.claimed_by.unwrap();
    assert!(claimed_by == "orchestrator-1" || claimed_by == "orchestrator-2");
}

/// Test that PendingRetry jobs are claimed before Created jobs (priority)
#[rstest]
#[tokio::test]
async fn test_claim_priority_pending_retry_first() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create two jobs: one Created, one PendingRetry (older)
    let job_created_id = Uuid::new_v4();
    job_created.id = job_created_id;
    let job_retry_id = Uuid::new_v4();
    job_created.id = job_retry_id;

    // Create the Created job first (older timestamp)
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    let job_created = JobItem::create(
        "test_created".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job_created).await.expect("Failed to create Created job");

    // Create the PendingRetry job after (newer timestamp)
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
    let job_retry = JobItem::create(
        "test_retry".to_string(),
        JobType::SnosRun,
        JobStatus::PendingRetry,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job_retry).await.expect("Failed to create PendingRetry job");

    // Claim a job - should get PendingRetry despite it being newer
    let claimed = db
        .claim_job_for_processing(&JobType::SnosRun, "test-orchestrator")
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    // Verify PendingRetry was claimed first
    assert_eq!(claimed.id, job_retry_id, "PendingRetry job should be claimed first");
    assert_eq!(claimed.status, JobStatus::LockedForProcessing);
}

/// Test that jobs with future available_at are not claimed
#[rstest]
#[tokio::test]
async fn test_claim_respects_available_at() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create a job available in the future
    let job_future_id = Uuid::new_v4();
    job_future.id = job_future_id;
    let mut job_future = JobItem::create(
        "test_future".to_string(),
        JobType::ProofCreation,
        JobStatus::PendingVerification,
        create_metadata_for_job_type(JobType::ProofCreation, 0),
    );
    job_future.metadata.common.available_at = Some(Utc::now() + chrono::Duration::seconds(3600)); // 1 hour from now
    db.create_job(job_future).await.expect("Failed to create future job");

    // Create a job available now
    let job_now_id = Uuid::new_v4();
    job_now.id = job_now_id;
    let mut job_now = JobItem::create(
        "test_now".to_string(),
        JobType::ProofCreation,
        JobStatus::PendingVerification,
        create_metadata_for_job_type(JobType::ProofCreation, 0),
    );
    job_now.metadata.common.available_at = Some(Utc::now() - chrono::Duration::seconds(10)); // Already available
    db.create_job(job_now).await.expect("Failed to create available job");

    // Claim a job for verification
    let claimed = db
        .claim_job_for_verification(&JobType::ProofCreation, "test-orchestrator")
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    // Verify the available job was claimed, not the future one
    assert_eq!(claimed.id, job_now_id, "Job with past available_at should be claimed");
    assert_eq!(claimed.status, JobStatus::LockedForProcessing);

    // Verify the future job is still unclaimed
    let future_job_status = db.get_job_by_id(job_future_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(future_job_status.status, JobStatus::PendingVerification);
    assert!(future_job_status.claimed_by.is_none());
}

/// Test that claimed_by is kept during LockedForProcessing status
#[rstest]
#[tokio::test]
async fn test_claim_retained_during_processing() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create and claim a job
    let job_id = Uuid::new_v4();
    job.id = job_id;
    let job = JobItem::create(
        "test_claim_retention".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job.clone()).await.expect("Failed to create job");

    let orchestrator_id = "test-orchestrator".to_string();
    let claimed_job = db
        .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    // Verify claim is set
    assert_eq!(claimed_job.claimed_by, Some(orchestrator_id.clone()));
    assert_eq!(claimed_job.status, JobStatus::LockedForProcessing);

    // Simulate processing (update job without clearing claim)
    let mut updated_job = claimed_job.clone();
    updated_job.metadata.common.process_started_at = Some(Utc::now());

    let update =
        crate::types::jobs::job_updates::JobItemUpdates::new().update_metadata(updated_job.metadata.clone()).build();

    db.update_job(&claimed_job, update).await.expect("Failed to update job");

    // Verify claim is still present
    let db_job = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(db_job.claimed_by, Some(orchestrator_id.clone()));
    assert_eq!(db_job.status, JobStatus::LockedForProcessing);
}

/// Test that oldest Created jobs within the same priority level are claimed first
#[rstest]
#[tokio::test]
async fn test_claim_oldest_first_within_priority() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create three Created jobs with slight delays to ensure different timestamps
    let job1_id = Uuid::new_v4();
    job1.id = job1_id;
    let job1 = JobItem::create(
        "test_oldest".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job1).await.expect("Failed to create job 1");

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let job2_id = Uuid::new_v4();
    job2.id = job2_id;
    let job2 = JobItem::create(
        "test_middle".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job2).await.expect("Failed to create job 2");

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let job3_id = Uuid::new_v4();
    job3.id = job3_id;
    let job3 = JobItem::create(
        "test_newest".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job3).await.expect("Failed to create job 3");

    // Claim jobs one by one
    let claimed1 = db
        .claim_job_for_processing(&JobType::SnosRun, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    let claimed2 = db
        .claim_job_for_processing(&JobType::SnosRun, "orch-2")
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    let claimed3 = db
        .claim_job_for_processing(&JobType::SnosRun, "orch-3")
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    // Verify they were claimed in order (oldest first)
    assert_eq!(claimed1.id, job1_id, "Oldest job should be claimed first");
    assert_eq!(claimed2.id, job2_id, "Second oldest job should be claimed second");
    assert_eq!(claimed3.id, job3_id, "Newest job should be claimed last");
}

/// Test that no jobs are claimed when all are in ineligible statuses
#[rstest]
#[tokio::test]
async fn test_no_claim_when_all_ineligible() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create jobs in various ineligible statuses
    let statuses =
        vec![JobStatus::LockedForProcessing, JobStatus::LockedForProcessing, JobStatus::Completed, JobStatus::Failed];

    for (i, status) in statuses.iter().enumerate() {
        let job = JobItem::create(
            Uuid::new_v4(),
            JobType::SnosRun,
            *status,
            format!("test_ineligible_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create job");
    }

    // Try to claim - should get None
    let claimed = db.claim_job_for_processing(&JobType::SnosRun, "test-orchestrator").await.expect("Failed to claim");

    assert!(claimed.is_none(), "No jobs should be claimed when all are ineligible");
}
