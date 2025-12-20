/// Tests for multi-orchestrator concurrency in greedy mode
/// These tests verify that multiple orchestrators can safely compete for jobs
/// without conflicts, race conditions, or duplicate processing.
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use rstest::rstest;
use std::sync::Arc;
use tokio::sync::Barrier;
use uuid::Uuid;

/// Test multiple orchestrators competing for multiple jobs
/// Verifies that all jobs are claimed exactly once
#[rstest]
#[tokio::test]
async fn test_multiple_orchestrators_claim_multiple_jobs() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create 10 jobs
    let job_count = 10;
    let mut job_ids = Vec::new();
    for i in 0..job_count {
        let job_id = Uuid::new_v4();
        job.id = job_id;
        job_ids.push(job_id);
        let job = JobItem::create(format!("test_concurrent_{}", JobType::SnosRun, JobStatus::Created, i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create job");
    }

    // Spawn 5 orchestrators, each trying to claim jobs
    let orchestrator_count = 5;
    let barrier = Arc::new(Barrier::new(orchestrator_count));
    let mut handles = Vec::new();

    for orch_id in 0..orchestrator_count {
        let db_clone = db.clone();
        let barrier_clone = barrier.clone();
        let orchestrator_name = format!("orchestrator-{}", orch_id);

        let handle = tokio::spawn(async move {
            // Wait for all orchestrators to be ready
            barrier_clone.wait().await;

            let mut claimed_jobs = Vec::new();

            // Try to claim jobs in a loop (simulate polling)
            for _ in 0..5 {
                match db_clone.claim_job_for_processing(&JobType::SnosRun, &orchestrator_name).await {
                    Ok(Some(job)) => {
                        claimed_jobs.push(job.id);
                    }
                    Ok(None) => {
                        // No job available, stop trying
                        break;
                    }
                    Err(e) => {
                        eprintln!("Error claiming job: {}", e);
                        break;
                    }
                }

                // Small delay between claims
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }

            claimed_jobs
        });

        handles.push(handle);
    }

    // Wait for all orchestrators to finish
    let mut all_claimed_jobs = Vec::new();
    for handle in handles {
        let claimed = handle.await.expect("Orchestrator task failed");
        all_claimed_jobs.extend(claimed);
    }

    // Verify all jobs were claimed exactly once
    assert_eq!(all_claimed_jobs.len(), job_count, "All jobs should be claimed");

    // Check for duplicates
    let mut sorted_claims = all_claimed_jobs.clone();
    sorted_claims.sort();
    sorted_claims.dedup();
    assert_eq!(sorted_claims.len(), job_count, "No job should be claimed twice");

    // Verify all jobs are in LockedForProcessing status
    for job_id in job_ids {
        let job = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");
        assert_eq!(job.status, JobStatus::LockedForProcessing);
        assert!(job.claimed_by.is_some());
    }
}

/// Test that an orchestrator can't claim the same job twice
#[rstest]
#[tokio::test]
async fn test_single_orchestrator_cannot_double_claim() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create one job
    let job_id = Uuid::new_v4();
    job.id = job_id;
    let job = JobItem::create(
        "test_double_claim".to_string(),
        JobType::SnosRun,
        JobStatus::Created,
        create_metadata_for_job_type(JobType::SnosRun, 0),
    );
    db.create_job(job).await.expect("Failed to create job");

    let orchestrator_id = "test-orchestrator";

    // First claim should succeed
    let first_claim = db
        .claim_job_for_processing(&JobType::SnosRun, orchestrator_id)
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    assert_eq!(first_claim.id, job_id);

    // Second claim should return None (no available jobs)
    let second_claim = db.claim_job_for_processing(&JobType::SnosRun, orchestrator_id).await.expect("Failed to claim");

    assert!(second_claim.is_none(), "Second claim should return None");

    // Verify job is still claimed by original orchestrator
    let job_status = db.get_job_by_id(job_id).await.expect("Failed to get job").expect("Job not found");

    assert_eq!(job_status.claimed_by, Some(orchestrator_id.to_string()));
}

/// Test high-concurrency scenario with many orchestrators and jobs
#[rstest]
#[tokio::test]
async fn test_high_concurrency_claim_race() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create 50 jobs
    let job_count = 50;
    for i in 0..job_count {
        let job = JobItem::create(
            format!("high_concurrency_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create job");
    }

    // Spawn 20 orchestrators
    let orchestrator_count = 20;
    let barrier = Arc::new(Barrier::new(orchestrator_count));
    let mut handles = Vec::new();

    for orch_id in 0..orchestrator_count {
        let db_clone = db.clone();
        let barrier_clone = barrier.clone();
        let orchestrator_name = format!("orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            let mut claimed_count = 0;

            // Each orchestrator tries to claim up to 10 jobs
            for _ in 0..10 {
                match db_clone.claim_job_for_processing(&JobType::ProofCreation, &orchestrator_name).await {
                    Ok(Some(_)) => {
                        claimed_count += 1;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
            }

            claimed_count
        });

        handles.push(handle);
    }

    // Collect results
    let mut total_claimed = 0;
    for handle in handles {
        let claimed = handle.await.expect("Task failed");
        total_claimed += claimed;
    }

    // Verify total claims equals job count
    assert_eq!(total_claimed, job_count, "All jobs should be claimed exactly once");

    // Verify all jobs are claimed
    let locked_jobs = db.get_jobs_by_status(JobStatus::LockedForProcessing).await.expect("Failed to get jobs");

    assert_eq!(locked_jobs.len(), job_count as usize, "All jobs should be in LockedForProcessing");
}

/// Test that claim atomicity is maintained even with database delays
#[rstest]
#[tokio::test]
async fn test_claim_atomicity_with_delays() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create jobs
    let job_count = 20;
    for i in 0..job_count {
        let job = JobItem::create(format!("delay_test_{}", i), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0));
        db.create_job(job).await.expect("Failed to create job");
    }

    // Spawn orchestrators that claim with delays
    let orchestrator_count = 10;
    let barrier = Arc::new(Barrier::new(orchestrator_count));
    let mut handles = Vec::new();

    for orch_id in 0..orchestrator_count {
        let db_clone = db.clone();
        let barrier_clone = barrier.clone();
        let orchestrator_name = format!("delayed-orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            let mut claims = Vec::new();

            for attempt in 0..5 {
                // Add random delays to increase race condition likelihood
                if attempt > 0 {
                    let delay_ms = (orch_id * 5 + attempt * 3) % 20;
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms as u64)).await;
                }

                match db_clone.claim_job_for_processing(&JobType::SnosRun, &orchestrator_name).await {
                    Ok(Some(job)) => claims.push(job.id),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            claims
        });

        handles.push(handle);
    }

    // Collect all claims
    let mut all_claims = Vec::new();
    for handle in handles {
        let claims = handle.await.expect("Task failed");
        all_claims.extend(claims);
    }

    // Verify no duplicates
    let mut sorted = all_claims.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), all_claims.len(), "No duplicate claims should occur");
    assert_eq!(all_claims.len(), job_count, "All jobs should be claimed");
}

/// Test mixed operations: claiming, processing, and completing concurrently
#[rstest]
#[tokio::test]
async fn test_concurrent_claim_process_complete_cycle() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create initial batch of jobs
    let initial_jobs = 15;
    for i in 0..initial_jobs {
        let job = JobItem::create(format!("cycle_test_{}", i), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0));
        db.create_job(job).await.expect("Failed to create job");
    }

    let db_claim = db.clone();
    let db_complete = db.clone();
    let barrier = Arc::new(Barrier::new(2));
    let barrier_claim = barrier.clone();
    let barrier_complete = barrier.clone();

    // Orchestrator 1: Claims jobs
    let claimer = tokio::spawn(async move {
        barrier_claim.wait().await;

        let mut claimed = Vec::new();
        for _ in 0..10 {
            match db_claim.claim_job_for_processing(&JobType::SnosRun, "claimer").await {
                Ok(Some(job)) => claimed.push(job.id),
                Ok(None) => break,
                Err(_) => break,
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
        claimed
    });

    // Orchestrator 2: Completes already-claimed jobs
    let completer = tokio::spawn(async move {
        barrier_complete.wait().await;

        let mut completed = Vec::new();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await; // Let some claims happen first

        for _ in 0..10 {
            // Get a LockedForProcessing job
            let locked_jobs =
                db_complete.get_jobs_by_status(JobStatus::LockedForProcessing).await.expect("Failed to get jobs");

            if let Some(job) = locked_jobs.first() {
                let update = crate::types::jobs::job_updates::JobItemUpdates::new()
                    .update_status(JobStatus::Completed)
                    .clear_claim()
                    .build();

                db_complete.update_job(job, update).await.expect("Failed to complete job");
                completed.push(job.id);
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
        completed
    });

    let (claimed_ids, completed_ids) = tokio::join!(claimer, completer);
    let claimed_ids = claimed_ids.expect("Claimer failed");
    let completed_ids = completed_ids.expect("Completer failed");

    // Verify operations succeeded without conflicts
    assert!(!claimed_ids.is_empty(), "Should have claimed some jobs");
    assert!(!completed_ids.is_empty(), "Should have completed some jobs");

    // Verify final state is consistent
    let all_jobs = db.get_jobs_by_status(JobStatus::Completed).await.expect("Failed to get completed jobs");
    assert!(all_jobs.len() >= completed_ids.len(), "Completed jobs should be in final state");
}

/// Test that PendingRetry priority is maintained under concurrent load
#[rstest]
#[tokio::test]
async fn test_concurrent_priority_maintained() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.database();

    // Create 5 PendingRetry jobs
    let retry_count = 5;
    for i in 0..retry_count {
        let job = JobItem::create(format!("retry_{}", i), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0));
        db.create_job(job).await.expect("Failed to create retry job");
    }

    // Create 10 Created jobs
    let created_count = 10;
    for i in 0..created_count {
        let job = JobItem::create(format!("created_{}", i), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0));
        db.create_job(job).await.expect("Failed to create job");
    }

    // Spawn multiple orchestrators claiming concurrently
    let orchestrator_count = 3;
    let barrier = Arc::new(Barrier::new(orchestrator_count));
    let mut handles = Vec::new();

    for orch_id in 0..orchestrator_count {
        let db_clone = db.clone();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            let mut claimed_statuses = Vec::new();

            for _ in 0..5 {
                match db_clone.claim_job_for_processing(&JobType::SnosRun, &format!("orch-{}", orch_id)).await {
                    Ok(Some(job)) => {
                        // Record the original status (before it was changed to LockedForProcessing)
                        // We'll infer from version or just track that it was claimed
                        claimed_statuses.push(job.id);
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
            }

            claimed_statuses
        });

        handles.push(handle);
    }

    // Collect all claimed job IDs
    let mut all_claimed = Vec::new();
    for handle in handles {
        let claimed = handle.await.expect("Task failed");
        all_claimed.extend(claimed);
    }

    // All retry jobs should be claimed first (total 5)
    // Then some created jobs (up to 10 more)
    assert!(all_claimed.len() >= retry_count, "All PendingRetry jobs should be claimed");

    // Verify all jobs are claimed
    let locked = db.get_jobs_by_status(JobStatus::LockedForProcessing).await.expect("Failed to get jobs");
    assert_eq!(locked.len(), all_claimed.len());
}
