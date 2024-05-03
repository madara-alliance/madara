use rstest::*;
use starknet::core::types::StateUpdate;

use std::fs::read_to_string;
// use std::io::{self, BufRead};
use serde_json::Error;
use std::sync::Arc;
use mockall::predicate::*;

use starknet_core::types::{BlockId, FieldElement, MaybePendingStateUpdate};
use crate::common::{
    get_or_init_config,
    default_job_item,
};

use da_client_interface::{DaVerificationStatus, MockDaClient};

use orchestrator::{
    config::{Config, MockConfig, MockMyStarknetProvider, MyStarknetProvider},
    jobs::{
        da_job::DaJob,
        types::{JobItem, JobStatus, JobType},
        Job
    },
};

#[fixture]
fn state_update() -> StateUpdate {
    let state_update_path = "test-utils/stateUpdate.json".to_owned();
    let contents = read_to_string(state_update_path).expect("Couldn't find or load that file.");

    let v: Result<StateUpdate, Error> = serde_json::from_str(&contents.as_str());

    let state_update: StateUpdate = match v {
        Ok(state_update) => state_update,
        Err(e) => panic!("Couldn't parse the JSON file: {}", e),
    };

    // let blob_data = state_update_to_blob_data(630872, state_update);
    state_update
}

#[fixture]
fn blob_data() -> Vec<FieldElement> {
    vec![FieldElement::ONE, FieldElement::ZERO]
}

#[rstest]
#[tokio::test]
async fn test_create_job(
    #[future] #[from(get_or_init_config)] config: &'static dyn Config,
) {
    let config = config.await;
    let job = DaJob.create_job(config, String::from("0")).await;
    assert!(job.is_ok());

    let job = job.unwrap();
    
    let job_type = job.job_type;
    assert_eq!(job_type, JobType::DataSubmission, "job_type should be DataSubmission");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
// #[should_panic]
#[tokio::test]
async fn test_verify_job(
    #[future] #[from(get_or_init_config)] config: &'static dyn Config,
    #[from(default_job_item)] job_item: JobItem,
) {
    let config = config.await;
    // let x: DaVerificationStatus = config.da_client().verify_inclusion(job_item.external_id.unwrap_string()?).await?.into();
    assert!(DaJob.verify_job(config, &job_item).await.is_err());
}

#[rstest]
#[should_panic]
#[tokio::test]
async fn test_process_job(
    #[future] #[from(get_or_init_config)] configg: &'static dyn Config,
    #[from(default_job_item)] job_item: JobItem,
    state_update: StateUpdate,
    blob_data: Vec<FieldElement>,
) {
    let mut mock_provider = MockMyStarknetProvider::new();
    let mut mock_da_client = MockDaClient::new();
    // let mock_provider = Arc::new(MockMyStarknetProvider::new());
    // let mock_da_client = Arc::new(MockDaClient::new());
    let mut mock_config = MockConfig::new();

    mock_config.expect_da_client().with().times(1).return_const(Box::new(mock_da_client));
    // mock_config.expect_starknet_client().times(1).returning(move || Arc::clone(&arc_mock_provider));


    // Mocking get_state_update to return a specific state update
    let expected_block_id = BlockId::Number(123);

    mock_provider.expect_get_state_update()
        .with(eq(expected_block_id))
        .times(1)
        .returning(move |_| 
            { 
                Ok( MaybePendingStateUpdate::Update(state_update.clone()))
            }
    );

    // Mocking publish_state_diff to return a specific external ID
    let external_id = "external_id_123".to_string();

    // mock_da_client.expect_publish_state_diff()
    //     .with(eq(blob_data.clone()))
    //     .times(1)
    //     .returning(move |_| Ok(external_id.clone()) );

    // Assuming DaJob implements the processing using the Config trait
    let job_processor = DaJob; // Assuming DaJob is initialized properly
    let result = job_processor.process_job(&mock_config, &job_item).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "external_id_123");
}
