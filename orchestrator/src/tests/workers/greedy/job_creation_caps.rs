/// Tests for job creation caps in greedy mode
/// These tests verify that job creation respects the configured caps for
/// max_concurrent_created_*_jobs to prevent MongoDB overflow.
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::batch::{AggregatorBatchStatus, SnosBatch, SnosBatchStatus};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::JobSpecificMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::aggregator::AggregatorJobTrigger;
use crate::worker::event_handler::triggers::proving::ProvingJobTrigger;
use crate::worker::event_handler::triggers::snos::SnosJobTrigger;
use crate::worker::event_handler::triggers::JobTrigger;
use rstest::rstest;
use std::sync::Arc;
use uuid::Uuid;

/// Test that SNOS job creation respects the cap
#[rstest]
#[tokio::test]
async fn test_snos_job_cap_respected() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    // Get the configured cap
    let cap = config_arc.service_config().max_concurrent_created_snos_jobs;

    // Create (cap - 1) existing SNOS jobs in Created status
    for i in 0..(cap - 1) {
        let job = JobItem::create(
            Uuid::new_v4(),
            JobType::SnosRun,
            JobStatus::Created,
            format!("existing_snos_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create existing job");
    }

    // Create multiple closed SNOS batches that need jobs
    for i in 0..5 {
        let batch = SnosBatch { id: format!("batch_{}", i), status: SnosBatchStatus::Closed, ..Default::default() };
        db.create_snos_batch(&batch).await.expect("Failed to create batch");
    }

    // Count jobs before trigger
    let jobs_before = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_before, cap - 1);

    // Run the trigger - should create only 1 job (to reach cap)
    let trigger = SnosJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    // Count jobs after trigger
    let jobs_after = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    // Should have created exactly 1 job to reach the cap
    assert_eq!(jobs_after, cap, "Should create jobs up to cap");

    // Run trigger again - should create no new jobs (at cap)
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    let jobs_final = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_final, cap, "Should not exceed cap");
}

/// Test that ProofCreation job creation respects the cap
#[rstest]
#[tokio::test]
async fn test_proving_job_cap_respected() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    // Get the configured cap
    let cap = config_arc.service_config().max_concurrent_created_proving_jobs;

    // Create (cap) existing ProofCreation jobs in Created status
    for i in 0..cap {
        let job = JobItem::create(format!("existing_proving_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create existing job");
    }

    // Create completed SNOS jobs without ProofCreation successors
    for i in 0..5 {
        let snos_job = JobItem::create(format!("snos_{}", i),
            Default::default(),
        );
        db.create_job(snos_job).await.expect("Failed to create SNOS job");
    }

    // Count jobs before trigger
    let jobs_before = db
        .count_jobs_by_type_and_statuses(&JobType::ProofCreation, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_before, cap);

    // Run the trigger - should create NO new jobs (already at cap)
    let trigger = ProvingJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    // Count jobs after trigger
    let jobs_after = db
        .count_jobs_by_type_and_statuses(&JobType::ProofCreation, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_after, cap, "Should not exceed cap");
}

/// Test that Aggregator job creation respects the cap
#[rstest]
#[tokio::test]
async fn test_aggregator_job_cap_respected() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    // Get the configured cap
    let cap = config_arc.service_config().max_concurrent_created_aggregator_jobs;

    // Create (cap - 2) existing Aggregator jobs in Created status
    for i in 0..(cap - 2) {
        let job = JobItem::create(format!("existing_agg_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create existing job");
    }

    // Create aggregator batches that need jobs
    for i in 0..10 {
        let batch = crate::types::batch::AggregatorBatch {
            id: format!("agg_batch_{}", i),
            status: AggregatorBatchStatus::Created,
            ..Default::default()
        };
        db.create_aggregator_batch(&batch).await.expect("Failed to create aggregator batch");
    }

    // Count jobs before trigger
    let jobs_before = db
        .count_jobs_by_type_and_statuses(&JobType::Aggregator, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_before, cap - 2);

    // Run the trigger - should create only 2 jobs (to reach cap)
    let trigger = AggregatorJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    // Count jobs after trigger
    let jobs_after = db
        .count_jobs_by_type_and_statuses(&JobType::Aggregator, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(jobs_after, cap, "Should create jobs up to cap");
}

/// Test that cap counts both Created AND PendingRetry jobs
#[rstest]
#[tokio::test]
async fn test_cap_counts_created_and_pending_retry() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    let cap = config_arc.service_config().max_concurrent_created_snos_jobs;

    // Create mix of Created and PendingRetry jobs totaling (cap - 1)
    let created_count = (cap - 1) / 2;
    let retry_count = (cap - 1) - created_count;

    for i in 0..created_count {
        let job = JobItem::create(format!("created_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create Created job");
    }

    for i in 0..retry_count {
        let job = JobItem::create(format!("retry_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create PendingRetry job");
    }

    // Create batches
    for i in 0..3 {
        let batch = SnosBatch { id: format!("batch_{}", i), status: SnosBatchStatus::Closed, ..Default::default() };
        db.create_snos_batch(&batch).await.expect("Failed to create batch");
    }

    // Verify count includes both statuses
    let total_count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(total_count, cap - 1);

    // Run trigger - should create exactly 1 job
    let trigger = SnosJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    let final_count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(final_count, cap, "Cap should count both Created and PendingRetry");
}

/// Test that completing jobs frees up cap space
#[rstest]
#[tokio::test]
async fn test_completing_jobs_frees_cap_space() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    let cap = config_arc.service_config().max_concurrent_created_snos_jobs;

    // Create jobs up to cap
    let mut job_ids = Vec::new();
    for i in 0..cap {
        let job_id = Uuid::new_v4();
        job.id = job_id;
        job_ids.push(job_id);
        let job = JobItem::create(format!("snos_{}", i), JobType::SnosRun, JobStatus::Created, create_metadata_for_job_type(JobType::SnosRun, 0));
        db.create_job(job).await.expect("Failed to create job");
    }

    // Verify at cap
    let count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");
    assert_eq!(count, cap);

    // Complete half the jobs
    let jobs_to_complete = cap / 2;
    for i in 0..jobs_to_complete {
        let job = db.get_job_by_id(job_ids[i as usize]).await.expect("Failed to get job").expect("Job not found");
        let update = crate::types::jobs::job_updates::JobItemUpdates::new().update_status(JobStatus::Completed).build();
        db.update_job(&job, update).await.expect("Failed to update job");
    }

    // Verify count decreased
    let count_after_completion = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(count_after_completion, cap - jobs_to_complete, "Completing jobs should free up cap space");

    // Create batches and run trigger - should create more jobs now
    for i in 0..jobs_to_complete {
        let batch = SnosBatch { id: format!("new_batch_{}", i), status: SnosBatchStatus::Closed, ..Default::default() };
        db.create_snos_batch(&batch).await.expect("Failed to create batch");
    }

    let trigger = SnosJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    // Should be back at cap
    let final_count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(final_count, cap, "Should refill to cap after jobs complete");
}

/// Test that LockedForProcessing jobs don't count toward cap
#[rstest]
#[tokio::test]
async fn test_locked_jobs_not_counted_in_cap() {
    dotenvy::from_filename_override("../.env.test").ok();

    let config = TestConfigBuilder::new().configure_database(crate::tests::config::ConfigType::Actual).build().await;

    let db = config.config.database();
    let config_arc = config.config.clone();

    let cap = config_arc.service_config().max_concurrent_created_snos_jobs;

    // Create some jobs in LockedForProcessing status (should NOT count toward cap)
    for i in 0..5 {
        let mut job = JobItem::create(format!("locked_{}", i),
            Default::default(),
        );
        job.claimed_by = Some(format!("orch-{}", i));
        db.create_job(job).await.expect("Failed to create locked job");
    }

    // Create (cap - 1) jobs in Created status
    for i in 0..(cap - 1) {
        let job = JobItem::create(format!("created_{}", i),
            Default::default(),
        );
        db.create_job(job).await.expect("Failed to create job");
    }

    // Count should be (cap - 1), not including LockedForProcessing
    let count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(count, cap - 1, "LockedForProcessing should not count toward cap");

    // Create batch and run trigger - should create 1 more job
    let batch = SnosBatch { id: "test_batch".to_string(), status: SnosBatchStatus::Closed, ..Default::default() };
    db.create_snos_batch(&batch).await.expect("Failed to create batch");

    let trigger = SnosJobTrigger {};
    trigger.run_worker_if_enabled(config_arc.clone()).await.expect("Trigger failed");

    let final_count = db
        .count_jobs_by_type_and_statuses(&JobType::SnosRun, &[JobStatus::Created, JobStatus::PendingRetry])
        .await
        .expect("Failed to count jobs");

    assert_eq!(final_count, cap, "Should create up to cap, ignoring LockedForProcessing jobs");
}
