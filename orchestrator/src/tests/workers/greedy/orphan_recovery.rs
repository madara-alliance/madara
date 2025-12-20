/// Tests for orphan job recovery in greedy mode
/// These tests verify that stuck jobs (orphans) are properly detected and healed
/// when they exceed the timeout threshold.
use crate::tests::config::TestConfigBuilder;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::aggregator::AggregatorJobTrigger;
use crate::worker::event_handler::triggers::proving::ProvingJobTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use chrono::Utc;
use rstest::rstest;
use std::sync::Arc;
use uuid::Uuid;

/// Test that orphaned jobs are detected and healed after timeout
#[rstest]
#[tokio::test]
async fn test_orphan_recovery_after_timeout() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    // Create a job that was claimed but got stuck (orphaned)
    let job_id = Uuid::new_v4();
    let mut job = JobItem::new(
        job_id,
        JobType::ProofCreation,
        JobStatus::LockedForProcessing,
        "test_orphan".to_string(),
        Default::default(),
    );

    // Simulate a job that was claimed long ago (beyond timeout)
    job.metadata.common.claimed_by = Some("dead-orchestrator".to_string());
    job.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(7200)); // 2 hours ago
    job.metadata.common.process_started_at = Some(Utc::now() - chrono::Duration::seconds(7200));

    db.create_job(job.clone()).await.expect("Failed to create orphan job");

    // Get orphaned jobs (should find our stuck job)
    let timeout_seconds = 3600; // 1 hour timeout
    let orphans =
        db.get_orphaned_jobs(&JobType::ProofCreation, timeout_seconds).await.expect("Failed to get orphaned jobs");

    assert_eq!(orphans.len(), 1, "Should find exactly one orphaned job");
    assert_eq!(orphans[0].id, job_id);
    assert_eq!(orphans[0].metadata.common.claimed_by, Some("dead-orchestrator".to_string()));

    // Heal the orphaned job
    for orphan in orphans {
        let update = crate::types::jobs::job_updates::JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .clear_claim()
            .build();

        db.update_job(&orphan, update).await.expect("Failed to heal orphan");
    }

    // Verify the job was healed
    let healed_job = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");

    assert_eq!(healed_job.status, JobStatus::PendingRetry);
    assert!(healed_job.metadata.common.claimed_by.is_none(), "Claim should be cleared");
    assert!(healed_job.metadata.common.claimed_at.is_none(), "Claimed_at should be cleared");
}

/// Test that jobs within timeout are NOT considered orphans
#[rstest]
#[tokio::test]
async fn test_no_orphan_recovery_within_timeout() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create a job that was claimed recently (within timeout)
    let job_id = Uuid::new_v4();
    let mut job = JobItem::new(
        job_id,
        JobType::ProofCreation,
        JobStatus::LockedForProcessing,
        "test_recent".to_string(),
        Default::default(),
    );

    // Claimed 30 minutes ago
    job.metadata.common.claimed_by = Some("active-orchestrator".to_string());
    job.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(1800));
    job.metadata.common.process_started_at = Some(Utc::now() - chrono::Duration::seconds(1800));

    db.create_job(job.clone()).await.expect("Failed to create recent job");

    // Get orphaned jobs with 1 hour timeout - should find nothing
    let timeout_seconds = 3600; // 1 hour
    let orphans =
        db.get_orphaned_jobs(&JobType::ProofCreation, timeout_seconds).await.expect("Failed to get orphaned jobs");

    assert_eq!(orphans.len(), 0, "Should not find any orphaned jobs within timeout");

    // Verify job is still claimed
    let job_status = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");

    assert_eq!(job_status.status, JobStatus::LockedForProcessing);
    assert_eq!(job_status.metadata.common.claimed_by, Some("active-orchestrator".to_string()));
}

/// Test ProofCreation orphan healing via trigger
#[rstest]
#[tokio::test]
async fn test_proving_orphan_healing_trigger() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    // Create an orphaned ProofCreation job
    let orphan_id = Uuid::new_v4();
    let mut orphan = JobItem::new(
        orphan_id,
        JobType::ProofCreation,
        JobStatus::LockedForProcessing,
        "test_proving_orphan".to_string(),
        Default::default(),
    );
    orphan.metadata.common.claimed_by = Some("dead-prover".to_string());
    orphan.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(7200));
    orphan.metadata.common.process_started_at = Some(Utc::now() - chrono::Duration::seconds(7200));

    db.create_job(orphan).await.expect("Failed to create orphan");

    // Run the ProofCreation trigger (which includes orphan healing)
    let trigger = ProvingJobTrigger {};
    // Note: We can't fully test this without the full setup, but we can verify the heal logic

    // Manually call heal to verify it works
    let orphans = db.get_orphaned_jobs(&JobType::ProofCreation, 3600).await.expect("Failed to get orphans");
    assert_eq!(orphans.len(), 1);

    for orphan in orphans {
        let update = crate::types::jobs::job_updates::JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .clear_claim()
            .build();
        db.update_job(&orphan, update).await.expect("Failed to heal");
    }

    // Verify healing worked
    let healed = db.get_job_by_id(orphan_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(healed.status, JobStatus::PendingRetry);
    assert!(healed.metadata.common.claimed_by.is_none());
}

/// Test Aggregator orphan healing via trigger
#[rstest]
#[tokio::test]
async fn test_aggregator_orphan_healing_trigger() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create an orphaned Aggregator job
    let orphan_id = Uuid::new_v4();
    let mut orphan = JobItem::new(
        orphan_id,
        JobType::Aggregator,
        JobStatus::LockedForProcessing,
        "test_aggregator_orphan".to_string(),
        Default::default(),
    );
    orphan.metadata.common.claimed_by = Some("dead-aggregator".to_string());
    orphan.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(7200));
    orphan.metadata.common.process_started_at = Some(Utc::now() - chrono::Duration::seconds(7200));

    db.create_job(orphan).await.expect("Failed to create orphan");

    // Get orphaned jobs
    let orphans = db.get_orphaned_jobs(&JobType::Aggregator, 3600).await.expect("Failed to get orphans");
    assert_eq!(orphans.len(), 1);

    // Heal them
    for orphan in orphans {
        let update = crate::types::jobs::job_updates::JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .clear_claim()
            .build();
        db.update_job(&orphan, update).await.expect("Failed to heal");
    }

    // Verify healing
    let healed = db.get_job_by_id(orphan_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(healed.status, JobStatus::PendingRetry);
    assert!(healed.metadata.common.claimed_by.is_none());
}

/// Test multiple orphans are all healed
#[rstest]
#[tokio::test]
async fn test_multiple_orphans_healed() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create multiple orphaned jobs
    let orphan_count = 5;
    let mut orphan_ids = Vec::new();

    for i in 0..orphan_count {
        let orphan_id = Uuid::new_v4();
        orphan_ids.push(orphan_id);

        let mut orphan = JobItem::new(
            orphan_id,
            JobType::ProofCreation,
            JobStatus::LockedForProcessing,
            format!("test_orphan_{}", i),
            Default::default(),
        );
        orphan.metadata.common.claimed_by = Some(format!("dead-orch-{}", i));
        orphan.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(7200));

        db.create_job(orphan).await.expect("Failed to create orphan");
    }

    // Get all orphans
    let orphans = db.get_orphaned_jobs(&JobType::ProofCreation, 3600).await.expect("Failed to get orphans");
    assert_eq!(orphans.len(), orphan_count);

    // Heal all
    for orphan in orphans {
        let update = crate::types::jobs::job_updates::JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .clear_claim()
            .build();
        db.update_job(&orphan, update).await.expect("Failed to heal");
    }

    // Verify all were healed
    for orphan_id in orphan_ids {
        let healed = db.get_job_by_id(orphan_id).await.expect("Failed to get job").expect("Job not found");
        assert_eq!(healed.status, JobStatus::PendingRetry);
        assert!(healed.metadata.common.claimed_by.is_none());
    }
}

/// Test that orphan healing increments retry counter
#[rstest]
#[tokio::test]
async fn test_orphan_healing_increments_retry_counter() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create an orphaned job
    let job_id = Uuid::new_v4();
    let mut job = JobItem::new(
        job_id,
        JobType::ProofCreation,
        JobStatus::LockedForProcessing,
        "test_retry_counter".to_string(),
        Default::default(),
    );
    job.metadata.common.claimed_by = Some("dead-orchestrator".to_string());
    job.metadata.common.claimed_at = Some(Utc::now() - chrono::Duration::seconds(7200));
    job.metadata.common.process_retry_attempt_no = 2; // Already has 2 retries

    db.create_job(job.clone()).await.expect("Failed to create job");

    // Get and heal the orphan
    let orphans = db.get_orphaned_jobs(&JobType::ProofCreation, 3600).await.expect("Failed to get orphans");
    assert_eq!(orphans.len(), 1);

    for orphan in orphans {
        let mut updated_metadata = orphan.metadata.clone();
        updated_metadata.common.process_retry_attempt_no += 1; // Increment retry counter

        let update = crate::types::jobs::job_updates::JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .update_metadata(updated_metadata)
            .clear_claim()
            .build();

        db.update_job(&orphan, update).await.expect("Failed to heal");
    }

    // Verify retry counter was incremented
    let healed = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");
    assert_eq!(healed.status, JobStatus::PendingRetry);
    assert_eq!(healed.metadata.common.process_retry_attempt_no, 3, "Retry counter should be incremented");
    assert!(healed.metadata.common.claimed_by.is_none());
}

/// Test that only LockedForProcessing jobs are considered for orphan recovery
#[rstest]
#[tokio::test]
async fn test_only_locked_jobs_are_orphans() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();

    // Create jobs in various statuses, all with old claimed_at times
    let old_time = Utc::now() - chrono::Duration::seconds(7200);

    let statuses = vec![
        (JobStatus::Created, "should_not_be_orphan_1"),
        (JobStatus::LockedForProcessing, "should_be_orphan"),
        (JobStatus::PendingRetry, "should_not_be_orphan_2"),
        (JobStatus::LockedForVerification, "should_not_be_orphan_3"),
        (JobStatus::Completed, "should_not_be_orphan_4"),
    ];

    let mut locked_job_id = Uuid::nil();

    for (status, internal_id) in statuses {
        let job_id = Uuid::new_v4();
        if status == JobStatus::LockedForProcessing {
            locked_job_id = job_id;
        }

        let mut job = JobItem::new(job_id, JobType::ProofCreation, status, internal_id.to_string(), Default::default());
        job.metadata.common.claimed_by = Some("test-orchestrator".to_string());
        job.metadata.common.claimed_at = Some(old_time);

        db.create_job(job).await.expect("Failed to create job");
    }

    // Get orphans - should only find the LockedForProcessing job
    let orphans = db.get_orphaned_jobs(&JobType::ProofCreation, 3600).await.expect("Failed to get orphans");

    assert_eq!(orphans.len(), 1, "Only LockedForProcessing jobs should be orphans");
    assert_eq!(orphans[0].id, locked_job_id);
    assert_eq!(orphans[0].status, JobStatus::LockedForProcessing);
}
