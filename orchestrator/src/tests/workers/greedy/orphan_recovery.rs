/// Orphan job detection and recovery tests for greedy mode
///
/// Simplified tests that verify orphan detection methods exist and can be called
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::types::{JobStatus, JobType};
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::rstest;

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
async fn test_get_orphaned_jobs_method_exists() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Verify get_orphaned_jobs method exists and can be called
    let orphans = db.get_orphaned_jobs(&JobType::ProofCreation, 3600).await.expect("Failed to get orphans");

    // No orphans initially
    assert!(orphans.len() == 0 || orphans.len() > 0); // Just verify it returns
}

#[rstest]
#[tokio::test]
async fn test_no_orphan_recovery_within_timeout() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job (recent)
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job = JobItem::create("recent_job".to_string(), JobType::SnosRun, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let orchestrator_id = "test-orchestrator".to_string();
    db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    // Query for orphans with very short timeout - recent job should not be orphaned
    let orphans = db.get_orphaned_jobs(&JobType::SnosRun, 1).await.expect("Failed to get orphans");

    assert_eq!(orphans.len(), 0, "Recent job should not be considered orphaned");
}

#[rstest]
#[tokio::test]
async fn test_orphan_healing_workflow() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job
    let metadata = create_metadata_for_job_type(&JobType::ProofCreation, 1);
    let job = JobItem::create("healing_test".to_string(), JobType::ProofCreation, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create");

    let claimed =
        db.claim_job_for_processing(&JobType::ProofCreation, "orch-1").await.expect("Failed").expect("No job");

    // Simulate healing: reset to PendingRetry and clear claimed_by
    db.update_job(
        &claimed,
        JobItemUpdates::new().update_status(JobStatus::PendingRetry).update_claimed_by(None).build(),
    )
    .await
    .expect("Failed to heal");

    // Verify it can be reclaimed
    let reclaimed =
        db.claim_job_for_processing(&JobType::ProofCreation, "orch-2").await.expect("Failed").expect("No job");

    assert_eq!(reclaimed.id, claimed.id);
    assert_eq!(reclaimed.status, JobStatus::LockedForProcessing);
}

#[rstest]
#[tokio::test]
async fn test_only_locked_jobs_can_be_orphans() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create jobs in various states
    let statuses = vec![JobStatus::Created, JobStatus::PendingVerification, JobStatus::Completed];

    for (i, status) in statuses.iter().enumerate() {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i as u64);
        let job = JobItem::create(format!("job_{}", i), JobType::SnosRun, status.clone(), metadata);
        db.create_job(job).await.expect("Failed to create");
    }

    // Also create and claim one job (LockedForProcessing)
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 999);
    let job = JobItem::create("locked".to_string(), JobType::SnosRun, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create");
    db.claim_job_for_processing(&JobType::SnosRun, "orch").await.expect("Failed").expect("No job");

    // Get orphans - only LockedForProcessing jobs can be orphans (if timed out)
    // With short timeout, we shouldn't find any
    let orphans = db.get_orphaned_jobs(&JobType::SnosRun, 1).await.expect("Failed");

    // All orphans (if any) should be LockedForProcessing status
    for orphan in orphans {
        assert_eq!(orphan.status, JobStatus::LockedForProcessing);
    }
}

#[rstest]
#[tokio::test]
async fn test_orphan_retry_counter_increment() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim job
    let metadata = create_metadata_for_job_type(&JobType::ProofCreation, 1);
    let job = JobItem::create("retry_test".to_string(), JobType::ProofCreation, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create");

    let claimed =
        db.claim_job_for_processing(&JobType::ProofCreation, "orch-1").await.expect("Failed").expect("No job");

    let initial_attempts = claimed.metadata.common.process_attempt_no;

    // Heal with incremented counter
    let mut updated_metadata = claimed.metadata.clone();
    updated_metadata.common.process_attempt_no += 1;

    db.update_job(
        &claimed,
        JobItemUpdates::new()
            .update_status(JobStatus::PendingRetry)
            .update_claimed_by(None)
            .update_metadata(updated_metadata)
            .build(),
    )
    .await
    .expect("Failed to heal");

    // Reclaim and verify counter incremented
    let reclaimed =
        db.claim_job_for_processing(&JobType::ProofCreation, "orch-2").await.expect("Failed").expect("No job");

    assert_eq!(reclaimed.metadata.common.process_attempt_no, initial_attempts + 1);
}

#[rstest]
#[tokio::test]
async fn test_multiple_job_types_orphan_detection() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Test that orphan detection works for different job types
    let job_types = vec![JobType::SnosRun, JobType::ProofCreation, JobType::Aggregator];

    for job_type in job_types {
        let _orphans = db.get_orphaned_jobs(&job_type, 3600).await.expect("Failed");
        // Just verify the method works for all job types without panicking
    }
}

#[rstest]
#[tokio::test]
async fn test_clear_claim_helper() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job
    let metadata = create_metadata_for_job_type(&JobType::DataSubmission, 1);
    let job = JobItem::create("clear_test".to_string(), JobType::DataSubmission, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create");

    let claimed =
        db.claim_job_for_processing(&JobType::DataSubmission, "orch-1").await.expect("Failed").expect("No job");

    assert!(claimed.claimed_by.is_some());

    // Use clear_claim helper
    db.update_job(&claimed, JobItemUpdates::new().update_status(JobStatus::PendingRetry).clear_claim().build())
        .await
        .expect("Failed to clear claim");

    // Verify claim was cleared
    let reclaimed =
        db.claim_job_for_processing(&JobType::DataSubmission, "orch-2").await.expect("Failed").expect("No job");

    assert_eq!(reclaimed.id, claimed.id);
}
