use std::sync::Arc;

use cairo_vm::vm::runners::cairo_pie::CairoPie;
use chrono::{SubsecRound, Utc};
use rstest::*;
use url::Url;
use uuid::Uuid;

use crate::core::client::alert::MockAlertClient;
use crate::tests::common::default_job_item;
use crate::tests::config::{MockType, TestConfigBuilder};
use crate::tests::jobs::ConfigType;
use crate::types::constant::{CAIRO_PIE_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::snos::SnosJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    // Create proper metadata structure
    let metadata =
        JobMetadata { common: CommonMetadata::default(), specific: JobSpecificMetadata::Snos(SnosMetadata::default()) };

    let job = SnosJobHandler.create_job(String::from("0"), metadata).await;

    assert!(job.is_ok());
    let job = job.unwrap();

    let job_type = job.job_type;
    assert_eq!(job_type, JobType::SnosRun, "job_type should be SnosRun");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn test_verify_job(#[from(default_job_item)] mut job_item: JobItem) {
    let services = TestConfigBuilder::new().build().await;

    // Update job_item to use the proper metadata structure for SNOS jobs
    job_item.metadata.specific = JobSpecificMetadata::Snos(SnosMetadata::default());

    let job_status = SnosJobHandler.verify_job(services.config.clone(), &mut job_item).await;

    assert!(
        matches!(job_status, Ok(JobVerificationStatus::Verified)),
        "Job verification status should be Verified or NotVerified"
    );
    //
    // // Should always be [Verified] for the moment.
    // assert_eq!(job_status, Ok(JobVerificationStatus::Verified));
}

/// We have a private pathfinder node used to run the Snos [prove_block] function.
/// It must be set or the test below will be ignored, since the Snos cannot run
/// without a Pathinder node for the moment.
pub const SNOS_PATHFINDER_RPC_URL_ENV: &str = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS";

#[rstest]
#[tokio::test(flavor = "multi_thread")]
async fn test_process_job() -> color_eyre::Result<()> {
    let pathfinder_url: Url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => url.parse()?,
        Err(_) => {
            println!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return Ok(());
        }
    };

    let services = TestConfigBuilder::new()
        .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(pathfinder_url)))
        .configure_storage_client(ConfigType::Actual)
        .build()
        .await;

    let storage_client = services.config.storage();

    // Create proper metadata structure
    let block_number = 2671680; // starknet sepolia block
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::Snos(SnosMetadata {
            snos_batch_index: 1,
            start_block: block_number,
            end_block: block_number,
            num_blocks: 1,
            full_output: true,
            cairo_pie_path: Some(format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME)),
            snos_output_path: Some(format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", block_number, PROGRAM_OUTPUT_FILE_NAME)),
            ..Default::default()
        }),
    };

    let mut job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: "1".into(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: String::new().into(),
        metadata,
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let result = SnosJobHandler.process_job(Arc::clone(&services.config), &mut job_item).await?;

    assert_eq!(result, "1"); // expecting "1" because it's the first batch

    let cairo_pie_key = format!("{}/{}", block_number, CAIRO_PIE_FILE_NAME);
    let snos_output_key = format!("{}/{}", block_number, SNOS_OUTPUT_FILE_NAME);

    let cairo_pie_data = storage_client.get_data(&cairo_pie_key).await?;
    let snos_output_data = storage_client.get_data(&snos_output_key).await?;

    // assert that we can build back the Pie & the Snos output
    let _ = CairoPie::from_bytes(&cairo_pie_data)?;
    let _: serde_json::Value = serde_json::from_slice(&snos_output_data)?;

    Ok(())
}

/// Test that check_snos_health returns false when SNOS RPC is unavailable.
/// This tests the core health check logic without needing to manipulate env vars.
#[rstest]
#[tokio::test]
async fn test_check_snos_health_unavailable() {
    use crate::worker::event_handler::jobs::snos::check_snos_health;

    // Use an invalid/unreachable URL to simulate SNOS being down
    let invalid_snos_url = Url::parse("http://127.0.0.1:1").unwrap();

    // check_snos_health should return false for unreachable URL
    let is_healthy = check_snos_health(&invalid_snos_url).await;

    assert!(!is_healthy, "Expected false when SNOS RPC is unavailable");
}

/// Test that check_snos_health returns true when SNOS RPC is available.
#[rstest]
#[tokio::test]
async fn test_check_snos_health_available() {
    use crate::worker::event_handler::jobs::snos::check_snos_health;

    let snos_url = match std::env::var(SNOS_PATHFINDER_RPC_URL_ENV) {
        Ok(url) => Url::parse(&url).unwrap(),
        Err(_) => {
            println!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
            return;
        }
    };

    // check_snos_health should return true for reachable URL
    let is_healthy = check_snos_health(&snos_url).await;

    assert!(is_healthy, "Expected true when SNOS RPC is available");
}

/// Test that when SNOS RPC is available, check_ready_to_process returns Ok
#[rstest]
#[tokio::test]
async fn test_check_ready_to_process_snos_available() -> color_eyre::Result<()> {
    // Skip if SNOS RPC URL is not set
    if std::env::var(SNOS_PATHFINDER_RPC_URL_ENV).is_err() {
        println!("Ignoring test: {} environment variable is not set", SNOS_PATHFINDER_RPC_URL_ENV);
        return Ok(());
    }

    // Mock alert client - no alert should be sent when SNOS is available
    let mut mock_alert_client = MockAlertClient::new();
    mock_alert_client.expect_send_message().times(0);

    let services = TestConfigBuilder::new()
        .configure_alerts(ConfigType::Mock(MockType::Alerts(Box::new(mock_alert_client))))
        .build()
        .await;

    // Call check_ready_to_process - should return Ok since SNOS is available
    let result = SnosJobHandler.check_ready_to_process(services.config.clone()).await;

    assert!(result.is_ok(), "Expected Ok when SNOS RPC is available");
    Ok(())
}
