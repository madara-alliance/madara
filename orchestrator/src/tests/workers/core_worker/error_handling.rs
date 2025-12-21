use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::Utc;

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

/// Test that release_job_claim with delay sets available_at correctly (60 seconds for processing failures)
#[tokio::test]
async fn test_release_claim_with_processing_delay() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job
    let metadata = create_metadata_for_job_type(&JobType::DataSubmission, 1);
    let job = JobItem::create("proc_delay_test".to_string(), JobType::DataSubmission, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let claimed = db
        .claim_job_for_processing(&JobType::DataSubmission, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job to claim");

    assert!(claimed.claimed_by.is_some());
    assert_eq!(claimed.status, JobStatus::LockedForProcessing);

    // Simulate processing failure by releasing the claim with 60-second delay
    let released = db.release_job_claim(claimed.id, Some(60)).await.expect("Failed to release claim");

    // Verify the claim was released
    assert!(released.claimed_by.is_none(), "Claim should be released");

    // Verify available_at is set to ~60 seconds in the future (with tolerance)
    assert!(released.available_at.is_some(), "available_at should be set for delayed retry");
    let delay = released.available_at.unwrap() - Utc::now();
    assert!(
        delay.num_seconds() >= 55 && delay.num_seconds() <= 65,
        "Delay should be ~60 seconds, got {} seconds",
        delay.num_seconds()
    );

    // Verify job cannot be claimed immediately
    let immediate_claim = db.claim_job_for_processing(&JobType::DataSubmission, "orch-2").await.expect("Failed");
    assert!(immediate_claim.is_none(), "Job should not be claimable immediately after release with delay");
}

/// Test that release_job_claim with verification delay sets available_at correctly (30 seconds)
#[tokio::test]
async fn test_release_claim_with_verification_delay() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create a job in PendingVerification status
    let metadata = create_metadata_for_job_type(&JobType::DataSubmission, 2);
    let job = JobItem::create("verif_delay_test".to_string(), JobType::DataSubmission, JobStatus::Processed, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let claimed = db
        .claim_job_for_verification(&JobType::DataSubmission, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job to claim");

    assert!(claimed.claimed_by.is_some());
    assert_eq!(claimed.status, JobStatus::Processed);

    // Simulate verification failure by releasing the claim with 30-second delay
    let released = db.release_job_claim(claimed.id, Some(30)).await.expect("Failed to release claim");

    // Verify the claim was released
    assert!(released.claimed_by.is_none(), "Claim should be released");

    // Verify available_at is set to ~30 seconds in the future (with tolerance)
    assert!(released.available_at.is_some(), "available_at should be set for delayed retry");
    let delay = released.available_at.unwrap() - Utc::now();
    assert!(
        delay.num_seconds() >= 25 && delay.num_seconds() <= 35,
        "Delay should be ~30 seconds, got {} seconds",
        delay.num_seconds()
    );

    // Verify job cannot be claimed immediately
    let immediate_claim = db.claim_job_for_verification(&JobType::DataSubmission, "orch-2").await.expect("Failed");
    assert!(immediate_claim.is_none(), "Job should not be claimable immediately after release with delay");
}

/// Test that release_job_claim without delay makes job immediately available
#[tokio::test]
async fn test_released_job_without_delay_immediately_available() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job
    let metadata = create_metadata_for_job_type(&JobType::DataSubmission, 4);
    let job = JobItem::create("nodelay_test".to_string(), JobType::DataSubmission, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let claimed = db
        .claim_job_for_processing(&JobType::DataSubmission, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job to claim");

    // Simulate JobHandlerService setting status to PendingRetry (what happens in real scenario)
    use crate::types::jobs::job_updates::JobItemUpdates;
    db.update_job(&claimed, JobItemUpdates::new().update_status(JobStatus::PendingRetryProcessing).build())
        .await
        .expect("Failed to update status");

    // Release the claim without delay (None)
    db.release_job_claim(claimed.id, None).await.expect("Failed to release claim");

    // Immediately try to claim again - should succeed
    let immediate_claim = db
        .claim_job_for_processing(&JobType::DataSubmission, "orch-2")
        .await
        .expect("Failed")
        .expect("Job should be immediately claimable");

    assert_eq!(immediate_claim.id, claimed.id, "Should claim the same job");
    assert_eq!(immediate_claim.claimed_by, Some("orch-2".to_string()), "Should be claimed by new orchestrator");
    assert!(immediate_claim.available_at.is_none(), "available_at should be cleared for immediate availability");
}

/// Test that claim release preserves job status and other fields
#[tokio::test]
async fn test_claim_release_preserves_job_state() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create a job with specific external_id
    let metadata = create_metadata_for_job_type(&JobType::ProofCreation, 5);
    let mut job =
        JobItem::create("state_test".to_string(), JobType::ProofCreation, JobStatus::Processed, metadata.clone());
    job.external_id = crate::types::jobs::external_id::ExternalId::String("ext-123".into());

    db.create_job(job.clone()).await.expect("Failed to create job");

    // Claim the job
    let claimed = db
        .claim_job_for_verification(&JobType::ProofCreation, "orch-1")
        .await
        .expect("Failed to claim")
        .expect("No job to claim");

    // Release the claim with delay
    let released = db.release_job_claim(claimed.id, Some(10)).await.expect("Failed to release claim");

    // Verify all fields are preserved except claimed_by, available_at, and version
    assert_eq!(released.id, job.id, "Job ID should be preserved");
    assert_eq!(released.internal_id, job.internal_id, "Internal ID should be preserved");
    assert_eq!(released.job_type, job.job_type, "Job type should be preserved");
    assert_eq!(released.status, JobStatus::Processed, "Status should be preserved");
    assert_eq!(released.external_id, job.external_id, "External ID should be preserved");
    assert_eq!(released.version, claimed.version + 1, "Version should be incremented on release");

    // Verify claim was cleared
    assert!(released.claimed_by.is_none(), "claimed_by should be cleared");

    // Verify available_at was set
    assert!(released.available_at.is_some(), "available_at should be set for delayed availability");
}

/// Test that release_job_claim returns error for non-existent job
#[tokio::test]
async fn test_claim_release_nonexistent_job_returns_error() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Try to release claim for a non-existent job
    let fake_id = uuid::Uuid::new_v4();
    let result = db.release_job_claim(fake_id, Some(60)).await;

    assert!(result.is_err(), "Should return error for non-existent job");

    // Verify it's the correct error type
    match result {
        Err(crate::core::client::database::DatabaseError::NoUpdateFound(_)) => {
            // Expected error type
        }
        Err(e) => panic!("Wrong error type: {:?}", e),
        Ok(_) => panic!("Should have returned an error"),
    }
}
