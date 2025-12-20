/// Tests for job retry limit enforcement
///
/// Verifies that jobs respect max_process_attempts and max_verification_attempts
/// before transitioning to Failed status.

use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::service::JobHandlerService;

async fn setup_test_config() -> crate::tests::config::TestConfigBuilderReturns {
    dotenvy::from_filename_override("../.env.test").ok();
    std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    std::env::set_var("MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", "0x0000000000000000000000000000000000000000");
    std::env::set_var("MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS", "0x0000000000000000000000000000000000000000");

    TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_settlement_client(ConfigType::Actual)
        .build()
        .await
}

/// Test that ProofRegistration jobs retry up to max_process_attempts (2) before failing
#[tokio::test]
async fn test_processing_retries_until_max_attempts() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // ProofRegistration has max_process_attempts = 2
    let job_type = JobType::ProofRegistration;
    let metadata = create_metadata_for_job_type(&job_type, 1);

    // Create a job
    let job = JobItem::create("retry_test_1".to_string(), job_type.clone(), JobStatus::Created, metadata);
    let job = db.create_job(job).await.expect("Failed to create job");

    assert_eq!(job.status, JobStatus::Created);
    assert_eq!(job.metadata.common.process_attempt_no, 0);

    // Attempt 1: Process and fail (will fail because we don't have real prover service)
    let result = JobHandlerService::process_job(job.id, config.config.clone()).await;
    
    // Should fail but move to PendingRetry (not Failed)
    assert!(result.is_err(), "Expected processing to fail without real services");
    
    let job_after_attempt1 = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    assert_eq!(job_after_attempt1.status, JobStatus::PendingRetry, "Should be PendingRetry after first failure");
    assert_eq!(job_after_attempt1.metadata.common.process_attempt_no, 1, "Should have 1 attempt");
    assert!(job_after_attempt1.claimed_by.is_none(), "Claim should be cleared");

    // Attempt 2: Process and fail again
    let result = JobHandlerService::process_job(job.id, config.config.clone()).await;
    assert!(result.is_err(), "Expected processing to fail again");
    
    let job_after_attempt2 = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    
    // Max attempts (2) reached - should now be Failed
    assert_eq!(job_after_attempt2.status, JobStatus::Failed, "Should be Failed after max attempts");
    assert_eq!(job_after_attempt2.metadata.common.process_attempt_no, 2, "Should have 2 attempts");
    assert!(job_after_attempt2.claimed_by.is_none(), "Claim should be cleared");
    assert!(job_after_attempt2.metadata.common.failure_reason.is_some(), "Should have failure reason");
    
    // Verify failure reason mentions max attempts
    let failure_reason = job_after_attempt2.metadata.common.failure_reason.unwrap();
    assert!(failure_reason.contains("attempts"), "Failure reason should mention attempts: {}", failure_reason);
}

/// Test that SNOS jobs fail immediately (max_process_attempts = 1)
#[tokio::test]
async fn test_snos_fails_immediately_no_retry() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // SnosRun has max_process_attempts = 1 (no retries)
    let job_type = JobType::SnosRun;
    let metadata = create_metadata_for_job_type(&job_type, 2);

    let job = JobItem::create("snos_no_retry_test".to_string(), job_type.clone(), JobStatus::Created, metadata);
    let job = db.create_job(job).await.expect("Failed to create job");

    assert_eq!(job.status, JobStatus::Created);
    assert_eq!(job.metadata.common.process_attempt_no, 0);

    // Attempt 1: Process and fail
    let result = JobHandlerService::process_job(job.id, config.config.clone()).await;
    assert!(result.is_err(), "Expected processing to fail");
    
    let job_after = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    
    // With max_process_attempts = 1, should go directly to Failed (no retry)
    assert_eq!(job_after.status, JobStatus::Failed, "Should be Failed immediately (max_attempts=1)");
    assert_eq!(job_after.metadata.common.process_attempt_no, 1, "Should have 1 attempt");
    assert!(job_after.claimed_by.is_none(), "Claim should be cleared");
}

/// Test processing attempt counter increments correctly
#[tokio::test]
async fn test_process_attempt_counter_increments() {
    let config = setup_test_config().await;
    let db = config.config.database();

    let job_type = JobType::ProofRegistration; // max_process_attempts = 2
    let metadata = create_metadata_for_job_type(&job_type, 3);

    let job = JobItem::create("counter_test".to_string(), job_type.clone(), JobStatus::Created, metadata);
    let job = db.create_job(job).await.expect("Failed to create job");

    assert_eq!(job.metadata.common.process_attempt_no, 0, "Initial attempt count should be 0");

    // First attempt
    let _ = JobHandlerService::process_job(job.id, config.config.clone()).await;
    let job = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    assert_eq!(job.metadata.common.process_attempt_no, 1, "Should be 1 after first attempt");
    assert_eq!(job.status, JobStatus::PendingRetry, "Should be PendingRetry after first failure");

    // Second attempt
    let _ = JobHandlerService::process_job(job.id, config.config.clone()).await;
    let job = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    assert_eq!(job.metadata.common.process_attempt_no, 2, "Should be 2 after second attempt");
    assert_eq!(job.status, JobStatus::Failed, "Should be Failed after max attempts");
}

/// Test that claim is properly cleared on retry
#[tokio::test]
async fn test_claim_cleared_on_retry() {
    let config = setup_test_config().await;
    let db = config.config.database();

    let job_type = JobType::ProofRegistration;
    let metadata = create_metadata_for_job_type(&job_type, 4);

    let job = JobItem::create("claim_clear_test".to_string(), job_type.clone(), JobStatus::Created, metadata);
    let job = db.create_job(job).await.expect("Failed to create job");

    // Claim the job
    let claimed = db.claim_job_for_processing(&job_type, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job to claim");

    assert_eq!(claimed.id, job.id);
    assert_eq!(claimed.claimed_by, Some("orch-1".to_string()));

    // Process and fail
    let _ = JobHandlerService::process_job(job.id, config.config.clone()).await;
    
    let job_after = db.get_job_by_id(job.id).await.expect("DB error").expect("Job not found");
    
    // Claim should be cleared
    assert!(job_after.claimed_by.is_none(), "Claim should be cleared after failure");
    assert_eq!(job_after.status, JobStatus::PendingRetry, "Should be PendingRetry");
    
    // Should be claimable by another orchestrator
    let reclaimed = db.claim_job_for_processing(&job_type, "orch-2")
        .await
        .expect("Failed to claim")
        .expect("Should be claimable again");
    
    assert_eq!(reclaimed.id, job.id, "Same job should be claimable");
    assert_eq!(reclaimed.claimed_by, Some("orch-2".to_string()), "Should be claimed by orch-2");
}
