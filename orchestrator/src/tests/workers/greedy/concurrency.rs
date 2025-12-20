/// Multi-orchestrator concurrency tests for greedy mode
///
/// Tests verify system behavior under high concurrent load with multiple
/// orchestrators competing for jobs.
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::rstest;
use std::collections::HashSet;

/// Helper to set up test config
async fn setup_test_config() -> crate::tests::config::TestConfigBuilderReturns {
    dotenvy::from_filename_override("../.env.test").ok();
    std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    std::env::set_var("MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", "0x0000000000000000000000000000000000000000");
    std::env::set_var("MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS", "0x0000000000000000000000000000000000000000");

    TestConfigBuilder::new()
        .configure_database(crate::tests::config::ConfigType::Actual)
        .configure_settlement_client(crate::tests::config::ConfigType::from(MockSettlementClient::new()))
        .configure_da_client(crate::tests::config::ConfigType::from(MockDaClient::new()))
        .configure_prover_client(crate::tests::config::ConfigType::from(MockProverClient::new()))
        .build()
        .await
}

#[rstest]
#[tokio::test]
async fn test_multiple_orchestrators_claim_multiple_jobs() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 10 jobs
    for i in 1..=10 {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i);
        let job = JobItem::create(format!("job_{}", i), JobType::SnosRun, JobStatus::Created, metadata);
        db.create_job(job).await.expect("Failed to create job");
    }

    // 5 orchestrators each try to claim jobs
    let mut handles = vec![];
    for orch_id in 0..5 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orchestrator-{}", orch_id);

        let handle = tokio::spawn(async move {
            let mut claimed_jobs = vec![];
            // Each orchestrator tries to claim up to 3 jobs
            for _ in 0..3 {
                if let Ok(Some(job)) =
                    config_clone.database().claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await
                {
                    claimed_jobs.push(job.id);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
            claimed_jobs
        });
        handles.push(handle);
    }

    // Collect all claimed job IDs
    let results = futures::future::join_all(handles).await;
    let all_claimed: Vec<_> = results.into_iter().filter_map(|r| r.ok()).flatten().collect();

    // Verify: All 10 jobs claimed, no duplicates
    let unique_jobs: HashSet<_> = all_claimed.iter().collect();
    assert_eq!(unique_jobs.len(), 10, "All 10 jobs should be claimed exactly once");
    assert_eq!(all_claimed.len(), 10, "No duplicate claims");
}

#[rstest]
#[tokio::test]
async fn test_single_orchestrator_cannot_double_claim() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 1 job
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job = JobItem::create("single_job".to_string(), JobType::SnosRun, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let orchestrator_id = "test-orchestrator".to_string();

    // First claim - should succeed
    let first_claim = db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed to claim");
    assert!(first_claim.is_some());

    // Second claim by same orchestrator - should return None (no available jobs)
    let second_claim = db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed to query");
    assert!(second_claim.is_none(), "Same orchestrator should not double-claim");
}

#[rstest]
#[tokio::test]
async fn test_high_concurrency_claim_race() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 50 jobs
    for i in 1..=50 {
        let metadata = create_metadata_for_job_type(&JobType::ProofCreation, i);
        let job = JobItem::create(format!("job_{}", i), JobType::ProofCreation, JobStatus::Created, metadata);
        db.create_job(job).await.expect("Failed to create job");
    }

    // 20 orchestrators race to claim jobs
    let mut handles = vec![];
    for orch_id in 0..20 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            let mut count = 0;
            // Each tries to claim up to 5 jobs
            for _ in 0..5 {
                if config_clone
                    .database()
                    .claim_job_for_processing(&JobType::ProofCreation, &orchestrator_id)
                    .await
                    .ok()
                    .flatten()
                    .is_some()
                {
                    count += 1;
                }
            }
            count
        });
        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    let total_claimed: usize = results.into_iter().filter_map(|r| r.ok()).sum();

    assert_eq!(total_claimed, 50, "Exactly 50 jobs should be claimed across all orchestrators");
}

#[rstest]
#[tokio::test]
async fn test_claim_atomicity_with_delays() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 5 jobs
    for i in 1..=5 {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i);
        let job = JobItem::create(format!("job_{}", i), JobType::SnosRun, JobStatus::Created, metadata);
        db.create_job(job).await.expect("Failed to create job");
    }

    // 10 orchestrators with random delays
    let mut handles = vec![];
    for orch_id in 0..10 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            // Random delay to simulate network latency
            tokio::time::sleep(tokio::time::Duration::from_millis((orch_id * 13) % 50)).await;

            config_clone
                .database()
                .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
                .await
                .ok()
                .flatten()
                .map(|j| j.id)
        });
        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    let claimed: Vec<_> = results.into_iter().filter_map(|r| r.ok()).filter_map(|x| x).collect();

    // Exactly 5 jobs claimed
    assert_eq!(claimed.len(), 5);

    // No duplicates
    let unique: HashSet<_> = claimed.iter().collect();
    assert_eq!(unique.len(), 5);
}

#[rstest]
#[tokio::test]
async fn test_concurrent_claim_process_complete_cycle() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 10 jobs
    for i in 1..=10 {
        let metadata = create_metadata_for_job_type(&JobType::DataSubmission, i);
        let job = JobItem::create(format!("job_{}", i), JobType::DataSubmission, JobStatus::Created, metadata);
        db.create_job(job).await.expect("Failed to create job");
    }

    // 3 orchestrators: claim, "process", complete
    let mut handles = vec![];
    for orch_id in 0..3 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            let mut completed = vec![];
            for _ in 0..5 {
                if let Ok(Some(job)) =
                    config_clone.database().claim_job_for_processing(&JobType::DataSubmission, &orchestrator_id).await
                {
                    // Simulate processing
                    tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

                    // Complete the job
                    let _ = config_clone.database()
                        .update_job(&job, JobItemUpdates::new().update_status(JobStatus::PendingVerification).build())
                        .await;
                    completed.push(job.id);
                }
            }
            completed
        });
        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    let all_completed: Vec<_> = results.into_iter().filter_map(|r| r.ok()).flatten().collect();

    // All 10 jobs should be completed
    assert_eq!(all_completed.len(), 10);

    // Verify no jobs left in Created or LockedForProcessing
    let remaining_created = db.get_jobs_by_status(JobStatus::Created).await.expect("Failed to query");
    let remaining_locked = db.get_jobs_by_status(JobStatus::LockedForProcessing).await.expect("Failed to query");

    assert_eq!(remaining_created.len(), 0);
    assert_eq!(remaining_locked.len(), 0);
}

#[rstest]
#[tokio::test]
async fn test_concurrent_priority_maintained() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 5 PendingRetry jobs and 5 Created jobs
    for i in 1..=5 {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i);
        let job = JobItem::create(format!("retry_{}", i), JobType::SnosRun, JobStatus::PendingRetry, metadata);
        db.create_job(job).await.expect("Failed to create retry job");
    }

    for i in 1..=5 {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i + 10);
        let job = JobItem::create(format!("new_{}", i), JobType::SnosRun, JobStatus::Created, metadata);
        db.create_job(job).await.expect("Failed to create new job");
    }

    // 5 orchestrators claim concurrently
    let mut handles = vec![];
    for orch_id in 0..5 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orch-{}", orch_id);

        let handle = tokio::spawn(async move {
            let job = config_clone
                .database()
                .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
                .await
                .ok()
                .flatten();
            job.map(|j| j.internal_id)
        });
        handles.push(handle);
    }

    let results = futures::future::join_all(handles).await;
    let claimed_ids: Vec<_> = results.into_iter().filter_map(|r| r.ok()).filter_map(|x| x).collect();

    // All 5 claimed jobs should be retry jobs (higher priority)
    assert_eq!(claimed_ids.len(), 5);
    for id in claimed_ids {
        assert!(id.starts_with("retry_"), "All claimed jobs should be PendingRetry jobs, got: {}", id);
    }
}
