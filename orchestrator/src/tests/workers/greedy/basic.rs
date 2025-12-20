/// Basic greedy mode tests to verify compilation and setup
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::create_metadata_for_job_type;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::{JobStatus, JobType};
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::rstest;

/// Test that a single orchestrator can successfully claim a job
#[rstest]
#[tokio::test]
async fn test_atomic_claim_single_orchestrator() {
    dotenvy::from_filename_override("../.env.test").ok();

    // Set required environment variables for test config builder
    std::env::set_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", "http://localhost:8545");
    std::env::set_var("MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", "0x0000000000000000000000000000000000000000");
    std::env::set_var("MADARA_ORCHESTRATOR_STARKNET_OPERATOR_ADDRESS", "0x0000000000000000000000000000000000000000");

    let config = TestConfigBuilder::new()
        .configure_database(crate::tests::config::ConfigType::Actual)
        .configure_settlement_client(crate::tests::config::ConfigType::from(MockSettlementClient::new()))
        .configure_da_client(crate::tests::config::ConfigType::from(MockDaClient::new()))
        .configure_prover_client(crate::tests::config::ConfigType::from(MockProverClient::new()))
        .build()
        .await;

    let db = config.config.database();

    // Create a test job in Created status
    let metadata = create_metadata_for_job_type(&JobType::SnosRun, 1);
    let job = JobItem::create("test_job_1".to_string(), JobType::SnosRun, JobStatus::Created, metadata);

    db.create_job(job).await.expect("Failed to create job");

    // Claim the job for processing
    let orchestrator_id = "test-orchestrator-1".to_string();
    let claimed_job = db
        .claim_job_for_processing(&JobType::SnosRun, &orchestrator_id)
        .await
        .expect("Failed to claim job")
        .expect("No job was claimed");

    // Verify the job was claimed
    assert_eq!(claimed_job.status, JobStatus::LockedForProcessing);
    assert_eq!(claimed_job.claimed_by, Some(orchestrator_id));
}
