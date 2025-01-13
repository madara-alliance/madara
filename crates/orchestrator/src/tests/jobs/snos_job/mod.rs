use std::collections::HashMap;
use std::sync::Arc;

use cairo_vm::vm::runners::cairo_pie::CairoPie;
use chrono::{SubsecRound, Utc};
use rstest::*;
use starknet_os::io::output::StarknetOsOutput;
use url::Url;
use uuid::Uuid;

use crate::constants::{CAIRO_PIE_FILE_NAME, SNOS_OUTPUT_FILE_NAME};
use crate::jobs::constants::JOB_METADATA_SNOS_BLOCK;
use crate::jobs::snos_job::SnosJob;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;
use crate::tests::common::default_job_item;
use crate::tests::config::{MockType, TestConfigBuilder};
use crate::tests::jobs::ConfigType;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let services = TestConfigBuilder::new().build().await;

    let job = SnosJob.create_job(services.config.clone(), String::from("0"), HashMap::new()).await;

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
    let job_status = SnosJob.verify_job(services.config.clone(), &mut job_item).await;

    // Should always be [Verified] for the moment.
    assert_eq!(job_status, Ok(JobVerificationStatus::Verified));
}

/// We have a private pathfinder node used to run the Snos [prove_block] function.
/// It must be set or the test below will be ignored, since the Snos cannot run
/// without a Pathinder node for the moment.
const SNOS_PATHFINDER_RPC_URL_ENV: &str = "MADARA_ORCHESTRATOR_RPC_FOR_SNOS";

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

    let mut job_item = JobItem {
        id: Uuid::new_v4(),
        internal_id: "1".into(),
        job_type: JobType::SnosRun,
        status: JobStatus::Created,
        external_id: String::new().into(),
        metadata: HashMap::from([(JOB_METADATA_SNOS_BLOCK.to_string(), "76793".to_string())]),
        version: 0,
        created_at: Utc::now().round_subsecs(0),
        updated_at: Utc::now().round_subsecs(0),
    };

    let result = SnosJob.process_job(Arc::clone(&services.config), &mut job_item).await?;

    assert_eq!(result, "76793");

    let cairo_pie_key = format!("76793/{}", CAIRO_PIE_FILE_NAME);
    let snos_output_key = format!("76793/{}", SNOS_OUTPUT_FILE_NAME);

    let cairo_pie_data = storage_client.get_data(&cairo_pie_key).await?;
    let snos_output_data = storage_client.get_data(&snos_output_key).await?;

    // assert that we can build back the Pie & the Snos output
    let _ = CairoPie::from_bytes(&cairo_pie_data)?;
    let _: StarknetOsOutput = serde_json::from_slice(&snos_output_data)?;

    Ok(())
}
