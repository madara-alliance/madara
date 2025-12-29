//! Self-healing tests for job workers
//!
//! These tests verify that orphaned jobs (stuck in LockedForProcessing status)
//! are automatically recovered and reset to Created status by the self-healing
//! mechanism in each worker trigger.
//!
//! All tests are marked with #[ignore] and contain TODO comments for future implementation.

#![allow(clippy::await_holding_lock)]

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::queue::MockQueueClient;
use crate::tests::common::test_utils::acquire_test_lock;
use crate::tests::config::TestConfigBuilder;
use crate::tests::workers::utils::get_job_item_mock_by_id;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::JobTrigger;
use httpmock::MockServer;
use mockall::predicate::eq;
use orchestrator_da_client_interface::MockDaClient;
use orchestrator_prover_client_interface::MockProverClient;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::rstest;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use std::error::Error;
use url::Url;
use uuid::Uuid;

/// Test self-healing for SNOS jobs
///
/// This test verifies that:
/// 1. Orphaned SNOS jobs (stuck in LockedForProcessing) are detected
/// 2. The timeout used is job-type specific (3600s for SNOS)
/// 3. Orphaned jobs are reset to Created status
/// 4. process_started_at timestamp is cleared
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_snos_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // 1. Create mock database with orphaned SNOS job
    // 2. Mock get_orphaned_jobs to return job with timeout=3600s (1 hour)
    // 3. Mock update_job to verify job is healed (status=Created, process_started_at=None)
    // 4. Run SnosJobTrigger.run_worker()
    // 5. Assert heal_orphaned_jobs was called with correct timeout
    // 6. Assert orphaned job was updated correctly

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();

    // Create an orphaned SNOS job
    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());
    let mut healed_job = orphaned_job.clone();
    healed_job.status = JobStatus::Created;
    healed_job.metadata.common.process_started_at = None;

    // TODO: Mock get_orphaned_jobs to return orphaned job
    db.expect_get_orphaned_jobs()
        .with(eq(JobType::SnosRun), eq(3600u64)) // 1 hour timeout for SNOS
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    // TODO: Mock update_job to verify healing
    // db.expect_update_job()...

    // TODO: Mock other required database calls for normal SNOS worker operation
    db.expect_get_latest_job_by_type().returning(|_| Ok(None));
    db.expect_get_oldest_job_by_type_excluding_statuses().returning(|_, _| Ok(None));
    db.expect_get_snos_batches_without_jobs().returning(|_, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .build()
        .await;

    // Run the SNOS worker
    let result = crate::worker::event_handler::triggers::snos::SnosJobTrigger.run_worker(services.config).await;

    assert!(result.is_ok());

    Ok(())
}

/// Test self-healing for Proving jobs
///
/// This test verifies that:
/// 1. Orphaned Proving jobs (stuck in LockedForProcessing) are detected
/// 2. The timeout used is job-type specific (1800s for ProofCreation)
/// 3. Orphaned jobs are reset to Created status
/// 4. process_started_at timestamp is cleared
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_proving_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // 1. Create mock database with orphaned Proving job
    // 2. Mock get_orphaned_jobs to return job with timeout=1800s (30 minutes)
    // 3. Mock update_job to verify job is healed
    // 4. Run ProvingJobTrigger.run_worker()
    // 5. Assert heal_orphaned_jobs was called with correct timeout
    // 6. Assert orphaned job was updated correctly

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    // Create an orphaned Proving job
    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());

    // TODO: Mock get_orphaned_jobs to return orphaned job
    db.expect_get_orphaned_jobs()
        .with(eq(JobType::ProofCreation), eq(1800u64)) // 30 minutes timeout for Proving
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    // TODO: Mock other required database calls
    db.expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    // Run the Proving worker
    let result = crate::worker::event_handler::triggers::proving::ProvingJobTrigger.run_worker(services.config).await;

    assert!(result.is_ok());

    Ok(())
}

/// Test self-healing for Proof Registration jobs
///
/// This test verifies that:
/// 1. Orphaned Proof Registration jobs are detected
/// 2. The timeout used is job-type specific (1800s for ProofRegistration)
/// 3. Orphaned jobs are reset to Created status
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_proof_registration_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // Similar to proving worker test but for ProofRegistration job type
    // Use timeout=1800s (30 minutes)

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());

    db.expect_get_orphaned_jobs()
        .with(eq(JobType::ProofRegistration), eq(1800u64)) // 30 minutes
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    db.expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    let result = crate::worker::event_handler::triggers::proof_registration::ProofRegistrationJobTrigger
        .run_worker(services.config)
        .await;

    assert!(result.is_ok());

    Ok(())
}

/// Test self-healing for Data Submission jobs
///
/// This test verifies that:
/// 1. Orphaned Data Submission jobs are detected
/// 2. The timeout used is job-type specific (1800s for DataSubmission)
/// 3. Orphaned jobs are reset to Created status
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_data_submission_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // Similar structure to other worker tests
    // Use timeout=1800s (30 minutes)

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());

    db.expect_get_orphaned_jobs()
        .with(eq(JobType::DataSubmission), eq(1800u64)) // 30 minutes
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    db.expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    let result = crate::worker::event_handler::triggers::data_submission_worker::DataSubmissionJobTrigger
        .run_worker(services.config)
        .await;

    assert!(result.is_ok());

    Ok(())
}

/// Test self-healing for State Transition jobs
///
/// This test verifies that:
/// 1. Orphaned State Transition jobs are detected
/// 2. The timeout used is job-type specific (2700s for StateTransition)
/// 3. Orphaned jobs are reset to Created status
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_state_transition_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // Similar structure to other worker tests
    // Use timeout=2700s (45 minutes)

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());

    db.expect_get_orphaned_jobs()
        .with(eq(JobType::StateTransition), eq(2700u64)) // 45 minutes
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    db.expect_get_jobs_without_successor().returning(|_, _, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    let result =
        crate::worker::event_handler::triggers::update_state::UpdateStateJobTrigger.run_worker(services.config).await;

    assert!(result.is_ok());

    Ok(())
}

/// Test self-healing for Aggregator jobs
///
/// This test verifies that:
/// 1. Orphaned Aggregator jobs are detected
/// 2. The timeout used is job-type specific (1800s for Aggregator)
/// 3. Orphaned jobs are reset to Created status
#[rstest]
#[tokio::test]
#[ignore = "TODO: Mock update_job expectations and verify heal_orphaned_jobs behavior"]
async fn test_aggregator_worker_self_healing() -> Result<(), Box<dyn Error>> {
    let _test_lock = acquire_test_lock();

    // TODO: Implement test logic
    // Similar structure to other worker tests
    // Use timeout=1800s (30 minutes)

    let server = MockServer::start();
    let da_client = MockDaClient::new();
    let mut db = MockDatabaseClient::new();
    let queue = MockQueueClient::new();
    let prover_client = MockProverClient::new();
    let settlement_client = MockSettlementClient::new();

    let orphaned_job = get_job_item_mock_by_id("1".to_string(), Uuid::new_v4());

    db.expect_get_orphaned_jobs()
        .with(eq(JobType::Aggregator), eq(1800u64)) // 30 minutes
        .returning(move |_, _| Ok(vec![orphaned_job.clone()]));

    db.expect_get_aggregator_batches_by_status().returning(|_, _| Ok(vec![]));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(&format!("http://localhost:{}", server.port())).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_database(db.into())
        .configure_queue_client(queue.into())
        .configure_da_client(da_client.into())
        .configure_prover_client(prover_client.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    let result =
        crate::worker::event_handler::triggers::aggregator::AggregatorJobTrigger.run_worker(services.config).await;

    assert!(result.is_ok());

    Ok(())
}

/// Test that different job types use different timeouts
///
/// This test verifies the ServiceParams::get_job_timeout() method
/// returns the correct timeout for each job type.
#[rstest]
#[tokio::test]
async fn test_job_type_specific_timeouts() -> Result<(), Box<dyn Error>> {
    use crate::types::params::service::ServiceParams;

    let service_params = ServiceParams {
        max_block_to_process: None,
        min_block_to_process: 0,
        max_concurrent_created_snos_jobs: 100,
        max_concurrent_snos_jobs: None,
        max_concurrent_proving_jobs: None,
        snos_job_timeout_seconds: 3600,           // 1 hour
        proving_job_timeout_seconds: 1800,        // 30 minutes
        proof_registration_timeout_seconds: 1800, // 30 minutes
        data_submission_timeout_seconds: 1800,    // 30 minutes
        state_transition_timeout_seconds: 2700,   // 45 minutes
        aggregator_job_timeout_seconds: 1800,     // 30 minutes
        snos_job_buffer_size: 50,
        max_priority_queue_size: 20,
    };

    assert_eq!(service_params.get_job_timeout(&JobType::SnosRun), 3600);
    assert_eq!(service_params.get_job_timeout(&JobType::ProofCreation), 1800);
    assert_eq!(service_params.get_job_timeout(&JobType::ProofRegistration), 1800);
    assert_eq!(service_params.get_job_timeout(&JobType::DataSubmission), 1800);
    assert_eq!(service_params.get_job_timeout(&JobType::StateTransition), 2700);
    assert_eq!(service_params.get_job_timeout(&JobType::Aggregator), 1800);

    Ok(())
}

/// End-to-end test for self-healing with actual database
///
/// This test uses an actual database (not mocked) to verify the complete
/// self-healing flow from detecting orphaned jobs to resetting them.
#[rstest]
#[tokio::test]
#[ignore = "TODO: Implement E2E test with actual database - create orphaned job, call heal_orphaned_jobs, verify healing"]
async fn test_self_healing_e2e_with_actual_database() -> Result<(), Box<dyn Error>> {
    // TODO: Implement E2E test
    // 1. Use TestConfigBuilder with ConfigType::Actual for database
    // 2. Create a job and set it to LockedForProcessing with old timestamp
    // 3. Call heal_orphaned_jobs directly
    // 4. Verify job was reset to Created status
    // 5. Verify process_started_at was cleared
    // 6. Use short timeout (2-5 seconds) for fast test execution

    use crate::tests::config::ConfigType;
    use crate::types::jobs::job_item::JobItem;

    let services = TestConfigBuilder::new().configure_database(ConfigType::Actual).build().await;

    let db = services.config.database();

    // TODO: Create orphaned job
    // TODO: Set process_started_at to old timestamp
    // TODO: Call heal_orphaned_jobs
    // TODO: Verify job was healed

    Ok(())
}
