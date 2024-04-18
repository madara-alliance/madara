use rstest::*;
use starknet::core::types::StateUpdate;

use std::fs::read_to_string;
// use std::io::{self, BufRead};
use serde_json::Error;

use crate::common::{
    get_or_init_config,
    default_job_item,
};

use orchestrator::{
    config::{MockConfig, Config},
    jobs::{
        da_job::DaJob,
        types::{JobItem, JobType},
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
    assert_eq!(job_type, JobType::DataSubmission);
    assert_eq!(job.metadata.values().len(), 0, "metadata should be empty");
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

    assert!(DaJob.verify_job(config, &job_item).await.is_err());
}

// #[rstest]
// #[should_panic]
// #[tokio::test]
// async fn test_process_job(
//     #[future] #[from(get_or_init_config)] config: &'static dyn Config,
//     #[from(default_job_item)] job_item: JobItem,
//     state_update: StateUpdate,
// ) {
//     let mut config = MockConfig::new();

//     // Mock the starknet_client to return a successful update
//     let mut mock_starknet_client = MockStarknetClient::new();
//     mock_starknet_client.expect_get_state_update()
//         .with(mockall::predicate::eq(BlockId::Number(1)))
//         .times(1)
//         .return_once(move |_| {
//             async move { Ok(MaybePendingStateUpdate::Update(state_update)) }.boxed()
//         });
    
//     // You need to return an Arc of the mocked client since your Config trait expects it
//     config.expect_starknet_client()
//         .times(1)
//         .returning(move || Arc::new(mock_starknet_client));

//     // Mock the da_client to return a fake external ID upon publishing state diff
//     let mut mock_da_client = MockDaClient::new();
//     mock_da_client.expect_publish_state_diff()
//         .withf(|blob_data| /* some condition to validate blob_data */ true)
//         .times(1)
//         .returning(|_| async { Ok("external_id".to_string()) }.boxed());
    
//     config.expect_da_client()
//         .times(1)
//         .returning(move || Arc::new(mock_da_client));

//     // Create a custom job item, assuming `default_job_item()` sets up a basic job item
//     let job_item = job_item;

//     let result = DaJob.process_job(&config, &job_item).await;
//     assert!(result.is_ok());
//     assert_eq!(result.unwrap(), "external_id");
// }

#[rstest]
fn test_max_process_attempts() {
    assert_eq!(DaJob.max_process_attempts(), 1);
}

#[rstest]
fn test_max_verification_attempts() {
    assert_eq!(DaJob.max_verification_attempts(), 3);
}

#[rstest]
fn test_verification_polling_delay_seconds() {
    assert_eq!(DaJob.verification_polling_delay_seconds(), 60);
}