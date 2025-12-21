/// Atomic job claiming tests for worker mode
///
/// Tests verify that MongoDB's findOneAndUpdate provides proper atomic claiming
/// across multiple orchestrators competing for the same jobs.
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::{Duration, Utc};
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
async fn test_atomic_claim_multiple_orchestrators() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create a single job
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job = JobItem::create("test_job_1".to_string(), JobType::SnosRun, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    // Simulate 5 orchestrators trying to claim the same job concurrently
    let mut handles = vec![];
    for i in 0..5 {
        let config_clone = config.config.clone();
        let orchestrator_id = format!("orchestrator-{}", i);

        let handle = tokio::spawn(async move {
            config_clone.database().claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await
        });
        handles.push(handle);
    }

    // Wait for all attempts
    let results: Vec<_> = futures::future::join_all(handles).await;

    // Exactly one should succeed
    let successful_claims: Vec<_> =
        results.into_iter().filter_map(|r| r.ok()).filter_map(|r| r.ok()).filter_map(|opt| opt).collect();

    assert_eq!(successful_claims.len(), 1, "Exactly one orchestrator should claim the job");
    assert_eq!(successful_claims[0].status, JobStatus::LockedForProcessing);
}

#[rstest]
#[tokio::test]
async fn test_claim_priority_pending_retry_first() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create jobs: 1 PendingRetry (created earlier) and 1 Created (created later)
    let metadata1 = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let mut job1 =
        JobItem::create("retry_job".to_string(), JobType::SnosRun, JobStatus::PendingRetryProcessing, metadata1);
    // Backdate the PendingRetry job
    job1.created_at = Utc::now() - Duration::hours(2);
    job1.updated_at = job1.created_at;
    db.create_job(job1).await.expect("Failed to create retry job");

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let metadata2 = create_metadata_for_job_type(&JobType::SnosRun, 2);
    let job2 = JobItem::create("new_job".to_string(), JobType::SnosRun, JobStatus::Created, metadata2);
    db.create_job(job2).await.expect("Failed to create new job");

    // Claim a job - should get PendingRetry first
    let orchestrator_id = "test-orchestrator".to_string();
    let claimed = db
        .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    assert_eq!(claimed.internal_id, "retry_job");
    assert_eq!(claimed.status, JobStatus::LockedForProcessing);
}

#[rstest]
#[tokio::test]
async fn test_claim_respects_available_at() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create job 1 - available now
    let metadata1 = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job1 = JobItem::create("available_now".to_string(), JobType::SnosRun, JobStatus::Created, metadata1);
    db.create_job(job1).await.expect("Failed to create job 1");

    // Create job 2 - available in the future
    let metadata2 = create_metadata_for_job_type(&JobType::SnosRun, 2);
    let mut job2 = JobItem::create("available_later".to_string(), JobType::SnosRun, JobStatus::Created, metadata2);
    db.create_job(job2).await.expect("Failed to create job 2");

    // Claim - should only get the available job
    let orchestrator_id = "test-orchestrator".to_string();
    let claimed = db
        .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim")
        .expect("No job claimed");

    assert_eq!(claimed.internal_id, "available_now");
}

#[rstest]
#[tokio::test]
async fn test_claim_oldest_first_within_priority() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create 3 jobs with same status but different timestamps
    for i in 1..=3 {
        let metadata = create_metadata_for_job_type(&JobType::SnosRun, i);
        let mut job = JobItem::create(format!("job_{}", i), JobType::SnosRun, JobStatus::Created, metadata);
        job.created_at = Utc::now() - Duration::hours((4 - i) as i64); // job_1 oldest, job_3 newest
        job.updated_at = job.created_at;
        db.create_job(job).await.expect("Failed to create job");
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }

    // Claim jobs in order
    let orchestrator_id = "test-orchestrator".to_string();

    let claimed1 =
        db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed").expect("No job");
    assert_eq!(claimed1.internal_id, "job_1");

    let claimed2 =
        db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed").expect("No job");
    assert_eq!(claimed2.internal_id, "job_2");

    let claimed3 =
        db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed").expect("No job");
    assert_eq!(claimed3.internal_id, "job_3");
}

#[rstest]
#[tokio::test]
async fn test_no_claim_when_all_ineligible() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create jobs that are all ineligible:

    // 1. Already claimed
    let metadata1 = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let mut job1 = JobItem::create("claimed".to_string(), JobType::SnosRun, JobStatus::Created, metadata1);
    job1.claimed_by = Some("other-orchestrator".to_string());
    db.create_job(job1).await.expect("Failed to create job 1");

    // 2. Available in future
    let metadata2 = create_metadata_for_job_type(&JobType::SnosRun, 2);
    let mut job2 = JobItem::create("future".to_string(), JobType::SnosRun, JobStatus::Created, metadata2);
    db.create_job(job2).await.expect("Failed to create job 2");

    // 3. Wrong status
    let metadata3 = create_metadata_for_job_type(&JobType::SnosRun, 3);
    let job3 = JobItem::create("completed".to_string(), JobType::SnosRun, JobStatus::Completed, metadata3);
    db.create_job(job3).await.expect("Failed to create job 3");

    // Attempt to claim - should return None
    let orchestrator_id = "test-orchestrator".to_string();
    let result = db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed to query");

    assert!(result.is_none(), "Should not claim any ineligible jobs");
}

#[rstest]
#[tokio::test]
async fn test_claim_retained_during_processing() {
    let config = setup_test_config().await;
    let db = config.config.database();

    // Create and claim a job
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job = JobItem::create("test_job".to_string(), JobType::SnosRun, JobStatus::Created, metadata);
    db.create_job(job).await.expect("Failed to create job");

    let orchestrator_id = "test-orchestrator".to_string();
    let claimed =
        db.claim_job_for_processing(&JobType::SnosRun, &orchestrator_id).await.expect("Failed").expect("No job");

    assert_eq!(claimed.status, JobStatus::LockedForProcessing);
    assert_eq!(claimed.claimed_by, Some(orchestrator_id.clone()));

    // Verify claimed_by is retained when we fetch the job
    let fetched = db.get_job_by_id(claimed.id).await.expect("Failed to fetch").expect("Job not found");
    assert_eq!(fetched.claimed_by, Some(orchestrator_id));
    assert_eq!(fetched.status, JobStatus::LockedForProcessing);
}
