use mockall::predicate::eq;
use rstest::*;
use settlement_client_interface::MockSettlementClient;

use std::{collections::HashMap, fs};

use super::super::common::init_config;

use crate::jobs::{
    constants::{
        JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY, JOB_METADATA_STATE_UPDATE_FETCH_FROM_TESTS,
        JOB_PROCESS_ATTEMPT_METADATA_KEY,
    },
    state_update_job::StateUpdateJob,
    types::{JobStatus, JobType},
    Job,
};

use httpmock::prelude::*;

#[rstest]
#[tokio::test]
async fn test_create_job() {
    let config = init_config(None, None, None, None, None, None).await;

    let job = StateUpdateJob.create_job(&config, String::from("0"), HashMap::default()).await;
    assert!(job.is_ok());

    let job = job.unwrap();
    let job_type = job.job_type;

    assert_eq!(job_type, JobType::StateTransition, "job_type should be StateTransition");
    assert!(!(job.id.is_nil()), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn test_process_job() {
    let server = MockServer::start();
    let mut settlement_client = MockSettlementClient::new();

    // Mock the latest block settled
    settlement_client.expect_get_last_settled_block().returning(|| Ok(651052_u64));

    // TODO: have tests for update_state_calldata, only kzg for now
    let block_numbers = ["651053", "651054", "651055", "651056"];
    for block_no in block_numbers {
        let program_output: Vec<[u8; 32]> = vec![];
        let block_proof: Vec<u8> = load_kzg_proof(block_no);
        let block_proof: [u8; 48] = block_proof.try_into().expect("test proof should be 48 bytes");
        settlement_client
            .expect_update_state_blobs()
            // TODO: vec![] is program_output
            .with(eq(program_output), eq(block_proof))
            .returning(|_, _| Ok(String::from("0x5d17fac98d9454030426606019364f6e68d915b91f6210ef1e2628cd6987442")));
    }

    let config = init_config(
        Some(format!("http://localhost:{}", server.port())),
        None,
        None,
        None,
        None,
        Some(settlement_client),
    )
    .await;

    let mut metadata: HashMap<String, String> = HashMap::new();
    metadata.insert(String::from(JOB_METADATA_STATE_UPDATE_FETCH_FROM_TESTS), String::from("TRUE"));
    metadata.insert(String::from(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY), block_numbers.join(","));
    metadata.insert(String::from(JOB_PROCESS_ATTEMPT_METADATA_KEY), String::from("0"));

    let mut job = StateUpdateJob.create_job(&config, String::from("internal_id"), metadata).await.unwrap();
    assert_eq!(StateUpdateJob.process_job(&config, &mut job).await.unwrap(), "651056".to_string())
}

#[rstest]
#[case(String::from("651052, 651054, 651051, 651056"), "numbers aren't sorted in increasing order")]
#[case(String::from("651052, 651052, 651052, 651052"), "Duplicated block numbers")]
#[case(String::from("a, 651054, b, 651056"), "settle list is not correctly formatted")]
#[case(String::from("651052, 651052, 651053, 651053"), "Duplicated block numbers")]
#[case(String::from(""), "settle list is not correctly formatted")]
#[tokio::test]
async fn test_process_job_invalid_inputs(#[case] block_numbers_to_settle: String, #[case] expected_error: &str) {
    let server = MockServer::start();
    let settlement_client = MockSettlementClient::new();
    let config = init_config(
        Some(format!("http://localhost:{}", server.port())),
        None,
        None,
        None,
        None,
        Some(settlement_client),
    )
    .await;

    let mut metadata: HashMap<String, String> = HashMap::new();
    metadata.insert(String::from(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY), block_numbers_to_settle);
    metadata.insert(String::from(JOB_PROCESS_ATTEMPT_METADATA_KEY), String::from("0"));

    let mut job = StateUpdateJob.create_job(&config, String::from("internal_id"), metadata).await.unwrap();
    let status = StateUpdateJob.process_job(&config, &mut job).await;
    assert!(status.is_err());

    if let Err(error) = status {
        let error_message = format!("{}", error);
        assert!(
            error_message.contains(expected_error),
            "Error message did not contain expected substring: {}",
            expected_error
        );
    }
}

#[rstest]
#[tokio::test]
#[should_panic(expected = "Gap detected between the first block to settle and the last one settle")]
async fn test_process_job_invalid_input_gap() {
    let server = MockServer::start();
    let mut settlement_client = MockSettlementClient::new();

    settlement_client.expect_get_last_settled_block().returning(|| Ok(4_u64));

    let config = init_config(
        Some(format!("http://localhost:{}", server.port())),
        None,
        None,
        None,
        None,
        Some(settlement_client),
    )
    .await;

    let mut metadata: HashMap<String, String> = HashMap::new();
    metadata.insert(String::from(JOB_METADATA_STATE_UPDATE_BLOCKS_TO_SETTLE_KEY), String::from("6, 7, 8"));
    metadata.insert(String::from(JOB_PROCESS_ATTEMPT_METADATA_KEY), String::from("0"));

    let mut job = StateUpdateJob.create_job(&config, String::from("internal_id"), metadata).await.unwrap();
    let _ = StateUpdateJob.process_job(&config, &mut job).await.unwrap();
}

// ==================== Utility functions ===========================

fn load_kzg_proof(block_no: &str) -> Vec<u8> {
    let file_path = format!("src/jobs/state_update_job/test_data/{}/kzg_proof.txt", block_no);
    let proof_str = fs::read_to_string(file_path).expect("Unable to read kzg_proof.txt").replace("0x", "");
    hex::decode(proof_str).unwrap()
}
