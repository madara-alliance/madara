use std::fs::read_to_string;
use std::path::PathBuf;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::storage::MockStorageClient;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::tests::common::default_job_item;
use crate::tests::config::{ConfigType, TestConfigBuilder};
use crate::types::batch::Batch;
use crate::types::constant::{
    BLOB_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME, STORAGE_ARTIFACTS_DIR, STORAGE_BLOB_DIR,
};
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::state_update::StateUpdateJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::utils::hex_string_to_u8_vec;
use assert_matches::assert_matches;
use bytes::Bytes;
use httpmock::prelude::*;
use lazy_static::lazy_static;
use mockall::predicate::{always, eq};
use num_bigint::BigUint;
use orchestrator_settlement_client_interface::MockSettlementClient;
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

lazy_static! {
    pub static ref CURRENT_PATH: PathBuf = std::env::current_dir().expect("Failed to get Current Path");
}

pub const X_0_FILE_NAME: &str = "x_0.txt";

// ================= Exhaustive tests (with minimum mock) =================

#[rstest]
#[tokio::test]
async fn test_process_job_attempt_not_present_fails() {
    let services = TestConfigBuilder::new().build().await;

    let mut job = default_job_item();

    // Update job metadata to use the proper structure
    job.metadata.specific = JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
        snos_output_paths: vec![],
        program_output_paths: vec![],
        blob_data_paths: vec![],
        tx_hashes: vec![],
        context: SettlementContext::Block(SettlementContextData { to_settle: vec![], last_failed: None }),
    });

    let res = StateUpdateJobHandler.process_job(services.config, &mut job).await.unwrap_err();
    assert!(
        matches!(res, JobError::StateUpdateJobError(StateUpdateError::BlockNumberNotFound)),
        "JobError should be StateUpdateJobError with BlockNumberNotFound"
    );
}

// TODO : make this test work
#[rstest]
#[case(None, vec![651053, 651054, 651055], 0)]
#[case(Some(651054), vec![651053, 651054, 651055], 1)]
#[tokio::test]
async fn test_process_job_works(
    #[case] failed_block_number: Option<u64>,
    #[case] blocks_to_process: Vec<u64>,
    #[case] processing_start_index: u8,
) {
    dotenvy::from_filename_override("../.env.test").expect("Failed to load the .env file");

    // Mocking the settlement client.
    let mut settlement_client = MockSettlementClient::new();
    let mut database_client = MockDatabaseClient::new();

    let last_block_to_process = blocks_to_process.last().unwrap().clone();

    // Mock the get batch by indexes method
    database_client.expect_get_batches_by_indexes().returning(move |_| {
        Ok(vec![Batch::new(0, last_block_to_process + 1, String::from(""), String::from(""), String::from(""))])
    });

    // This must be the last block number and should be returned as an output from the process job.
    let last_block_number = blocks_to_process[blocks_to_process.len() - 1];

    // Adding expectations for each block number to be called by settlement client.
    for block in blocks_to_process.iter().skip(processing_start_index as usize) {
        // Getting the blob data from file.
        let blob_data = read_to_string(
            CURRENT_PATH.join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block, BLOB_DATA_FILE_NAME)),
        )
        .unwrap();

        let blob_data_vec = vec![hex_string_to_u8_vec(&blob_data).unwrap()];

        // Getting the program output data from file.
        let program_output_data_vec = read_file_to_vec_u8_32(
            CURRENT_PATH
                .join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block, PROGRAM_OUTPUT_FILE_NAME))
                .to_str()
                .unwrap(),
        )
        .unwrap();

        settlement_client
            .expect_update_state_with_blobs()
            .with(eq(program_output_data_vec), eq(blob_data_vec), always(), always())
            .times(1)
            .returning(|_, _, _, _| Ok("0xbeef".to_string()));

        settlement_client.expect_wait_for_tx_finality().with(eq("0xbeef")).times(1).returning(|_| Ok(Some(1)));
    }

    settlement_client.expect_get_last_settled_block().with().returning(move || Ok(Some(651052)));
    // Setting random nonce
    settlement_client.expect_get_nonce().with().returning(move || Ok(2));

    // Building a temp config that will be used by `fetch_blob_data_for_block` and
    // `fetch_snos_for_block` functions while fetching the blob data from storage client.
    let services = TestConfigBuilder::new()
        .configure_storage_client(ConfigType::Actual)
        .configure_settlement_client(settlement_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    let storage_client = services.config.storage();

    // Prepare vectors to collect paths for metadata
    let mut snos_output_paths = Vec::new();
    let mut program_output_paths = Vec::new();
    let mut blob_data_paths = Vec::new();

    for block in blocks_to_process.iter() {
        // Getting the blob data from file.
        let blob_data_key = format!("{}/batch/{}", STORAGE_BLOB_DIR, block);
        let blob_data = read_to_string(
            CURRENT_PATH.join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block, BLOB_DATA_FILE_NAME)),
        )
        .unwrap();
        let blob_data_vec = hex_string_to_u8_vec(&blob_data).unwrap();

        // Getting the snos data from file.
        let snos_output_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block, SNOS_OUTPUT_FILE_NAME);
        let snos_output_data = read_to_string(
            CURRENT_PATH.join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block, SNOS_OUTPUT_FILE_NAME)),
        )
        .unwrap();

        // Getting the program output data from file.
        let program_output_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block, PROGRAM_OUTPUT_FILE_NAME);
        let program_output_data = read_file_to_vec_u8_32(
            CURRENT_PATH
                .join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block, PROGRAM_OUTPUT_FILE_NAME))
                .to_str()
                .unwrap(),
        )
        .unwrap();
        let program_output_data_serialized = bincode::serialize(&program_output_data).unwrap();

        storage_client.put_data(Bytes::from(snos_output_data), &snos_output_key).await.unwrap();
        storage_client.put_data(Bytes::from(blob_data_vec), &format!("{}/1.txt", blob_data_key)).await.unwrap();
        storage_client.put_data(Bytes::from(program_output_data_serialized), &program_output_key).await.unwrap();

        // Add paths to our vectors for metadata
        snos_output_paths.push(snos_output_key);
        program_output_paths.push(program_output_key);
        blob_data_paths.push(blob_data_key);
    }

    // Create proper metadata structure with the collected paths
    let mut metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths,
            program_output_paths,
            blob_data_paths,
            tx_hashes: Vec::new(), // Start with empty tx_hashes, they'll be populated during processing
            context: SettlementContext::Block(SettlementContextData {
                to_settle: blocks_to_process.clone(),
                last_failed: failed_block_number,
            }),
        }),
    };

    // Add process attempt to common metadata
    metadata.common.process_attempt_no = 0;

    // creating a `JobItem`
    let mut job = default_job_item();
    job.job_type = JobType::StateTransition;
    job.metadata = metadata;

    let res = StateUpdateJobHandler.process_job(services.config, &mut job).await.unwrap();
    assert_eq!(res, last_block_number.to_string());
}

// ==================== Mock Tests (Unit tests) ===========================

#[rstest]
#[tokio::test]
async fn create_job_works() {
    // Create proper metadata structure
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths: vec![format!("1/{}", SNOS_OUTPUT_FILE_NAME)],
            program_output_paths: vec![format!("1/{}", PROGRAM_OUTPUT_FILE_NAME)],
            blob_data_paths: vec![format!("1/{}", BLOB_DATA_FILE_NAME)],
            tx_hashes: vec![],
            context: SettlementContext::Block(SettlementContextData { to_settle: vec![1], last_failed: None }),
        }),
    };

    let job = StateUpdateJobHandler.create_job(String::from("0"), metadata).await;
    assert!(job.is_ok());

    let job = job.unwrap();
    let job_type = job.job_type;

    assert_eq!(job_type, JobType::StateTransition, "job_type should be StateTransition");
    assert!(!job.id.is_nil(), "id should not be nil");
    assert_eq!(job.status, JobStatus::Created, "status should be Created");
    assert_eq!(job.version, 0_i32, "version should be 0");
    assert_eq!(job.external_id.unwrap_string().unwrap(), String::new(), "external_id should be empty string");
}

#[rstest]
#[tokio::test]
async fn process_job_works_unit_test() {
    // Mocking settlement and storage client
    let mut settlement_client = MockSettlementClient::new();
    let mut storage_client = MockStorageClient::new();
    let mut database_client = MockDatabaseClient::new();

    // Mock the get batch by indexes method
    database_client
        .expect_get_batches_by_indexes()
        .returning(|_| Ok(vec![Batch::new(0, 651057, String::from(""), String::from(""), String::from(""))]));

    // Mock the latest block settled
    settlement_client.expect_get_last_settled_block().returning(|| Ok(Some(651052_u64)));

    let block_numbers = ["651053", "651054", "651055", "651056"];
    for block_no in block_numbers {
        let _state_diff: Vec<u8> = load_state_diff_file(block_no.parse::<u64>().unwrap()).await;

        // Setting expectation for SNOS output
        let snos_output_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block_no, SNOS_OUTPUT_FILE_NAME);
        let snos_output_data = read_to_string(
            CURRENT_PATH
                .join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block_no, SNOS_OUTPUT_FILE_NAME)),
        )
        .expect("Failed to read the snos output data json file");
        storage_client
            .expect_get_data()
            .with(eq(snos_output_key.clone()))
            .returning(move |_| Ok(Bytes::from(snos_output_data.clone())));

        // Setting expectation for blob data
        let blob_data_key = format!("{}/batch/{}", STORAGE_BLOB_DIR, block_no);
        let blob_data = read_to_string(
            CURRENT_PATH
                .join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block_no, BLOB_DATA_FILE_NAME)),
        )
        .expect("Failed to read the blob data txt file");
        let blob_data_vec = hex_string_to_u8_vec(&blob_data).unwrap();
        let blob_data_vec_clone = blob_data_vec.clone();
        storage_client
            .expect_get_data()
            .with(eq(format!("{}/1.txt", blob_data_key)))
            .returning(move |_| Ok(Bytes::from(blob_data_vec.clone())));
        storage_client
            .expect_list_files_in_dir()
            .with(eq(blob_data_key.clone()))
            .returning(move |_| Ok(vec![format!("{}/1.txt", blob_data_key)]));

        // Setting expectation for program output
        let program_output_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block_no, PROGRAM_OUTPUT_FILE_NAME);
        let program_output = read_file_to_vec_u8_32(
            CURRENT_PATH
                .join(format!("src/tests/jobs/state_update_job/test_data/{}/{}", block_no, PROGRAM_OUTPUT_FILE_NAME))
                .to_str()
                .unwrap(),
        )
        .unwrap();
        let program_output_clone = program_output.clone();
        storage_client
            .expect_get_data()
            .with(eq(program_output_key.clone()))
            .returning(move |_| Ok(Bytes::from(bincode::serialize(&program_output).unwrap())));

        // Setting expectation for nonce
        settlement_client.expect_get_nonce().returning(|| Ok(1));

        // Setting expectation for update_state_with_blobs
        let deserialized_program_output: Vec<[u8; 32]> =
            bincode::deserialize(&bincode::serialize(&program_output_clone).unwrap()).unwrap();
        settlement_client
            .expect_update_state_with_blobs()
            .with(eq(deserialized_program_output), eq(vec![blob_data_vec_clone]), always(), always())
            .returning(|_, _, _, _| {
                Ok(String::from("0x5d17fac98d9454030426606019364f6e68d915b91f6210ef1e2628cd6987442"))
            });

        // mocking the finality too
        settlement_client
            .expect_wait_for_tx_finality()
            .with(eq("0x5d17fac98d9454030426606019364f6e68d915b91f6210ef1e2628cd6987442"))
            .returning(|_| Ok(Some(1)));
    }

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_storage_client(storage_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    // Create proper metadata structure
    let mut metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths: block_numbers
                .iter()
                .map(|block| format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block, SNOS_OUTPUT_FILE_NAME))
                .collect(),
            blob_data_paths: block_numbers
                .iter()
                .map(|block| format!("{}/batch/{}", STORAGE_BLOB_DIR, block))
                .collect(),
            program_output_paths: block_numbers
                .iter()
                .map(|block| format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, block, PROGRAM_OUTPUT_FILE_NAME))
                .collect(),
            tx_hashes: vec![],
            context: SettlementContext::Block(SettlementContextData {
                to_settle: block_numbers.iter().map(|b| b.parse::<u64>().unwrap()).collect(),
                last_failed: None,
            }),
        }),
    };

    // Add process attempt to common metadata
    metadata.common.process_attempt_no = 0;

    let mut job = StateUpdateJobHandler.create_job(String::from("internal_id"), metadata).await.unwrap();
    assert_eq!(StateUpdateJobHandler.process_job(services.config, &mut job).await.unwrap(), "651056".to_string())
}

#[rstest]
#[case(vec![651052, 651054, 651051, 651056], "numbers aren't sorted in increasing order")]
#[case(vec![651052, 651052, 651052, 651052], "Duplicated block numbers")]
#[case(vec![651052, 651052, 651053, 651053], "Duplicated block numbers")]
#[tokio::test]
async fn process_job_invalid_inputs_errors(#[case] block_numbers: Vec<u64>, #[case] expected_error: &str) {
    let server = MockServer::start();
    let settlement_client = MockSettlementClient::new();

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_starknet_client(provider.into())
        .build()
        .await;

    // Create paths for each block number
    let snos_output_paths = block_numbers.iter().map(|block| format!("{}/{}", block, SNOS_OUTPUT_FILE_NAME)).collect();

    let program_output_paths =
        block_numbers.iter().map(|block| format!("{}/{}", block, PROGRAM_OUTPUT_FILE_NAME)).collect();

    let blob_data_paths = block_numbers.iter().map(|block| format!("{}/{}", block, BLOB_DATA_FILE_NAME)).collect();

    // Create proper metadata structure with invalid block numbers but valid paths
    let metadata = JobMetadata {
        common: CommonMetadata { process_attempt_no: 0, ..CommonMetadata::default() },
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths,
            program_output_paths,
            blob_data_paths,
            tx_hashes: vec![],
            context: SettlementContext::Block(SettlementContextData { to_settle: block_numbers, last_failed: None }),
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(String::from("internal_id"), metadata).await.unwrap();
    let status = StateUpdateJobHandler.process_job(services.config, &mut job).await;
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
async fn process_job_invalid_input_gap_panics() {
    let server = MockServer::start();
    let mut settlement_client = MockSettlementClient::new();

    settlement_client.expect_get_last_settled_block().returning(|| Ok(Some(4_u64)));

    let provider = JsonRpcClient::new(HttpTransport::new(
        Url::parse(format!("http://localhost:{}", server.port()).as_str()).expect("Failed to parse URL"),
    ));

    let services = TestConfigBuilder::new()
        .configure_starknet_client(provider.into())
        .configure_settlement_client(settlement_client.into())
        .build()
        .await;

    // Create proper metadata structure with valid paths
    let metadata = JobMetadata {
        common: CommonMetadata { process_attempt_no: 0, ..CommonMetadata::default() },
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths: vec![
                format!("{}/{}", 6, SNOS_OUTPUT_FILE_NAME),
                format!("{}/{}", 7, SNOS_OUTPUT_FILE_NAME),
                format!("{}/{}", 8, SNOS_OUTPUT_FILE_NAME),
            ],
            program_output_paths: vec![
                format!("{}/{}", 6, PROGRAM_OUTPUT_FILE_NAME),
                format!("{}/{}", 7, PROGRAM_OUTPUT_FILE_NAME),
                format!("{}/{}", 8, PROGRAM_OUTPUT_FILE_NAME),
            ],
            blob_data_paths: vec![
                format!("{}/{}", 6, BLOB_DATA_FILE_NAME),
                format!("{}/{}", 7, BLOB_DATA_FILE_NAME),
                format!("{}/{}", 8, BLOB_DATA_FILE_NAME),
            ],
            tx_hashes: vec![],
            context: SettlementContext::Block(SettlementContextData {
                to_settle: vec![6, 7, 8], // Gap between 4 and 6
                last_failed: None,
            }),
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(String::from("internal_id"), metadata).await.unwrap();
    let response = StateUpdateJobHandler.process_job(services.config, &mut job).await;

    assert_matches!(response,
        Err(e) => {
            let err = StateUpdateError::GapBetweenFirstAndLastBlock;
            let expected_error = JobError::StateUpdateJobError(err);
            assert_eq!(e.to_string(), expected_error.to_string());
        }
    );
}

// ==================== Utility functions ===========================

async fn load_state_diff_file(block_no: u64) -> Vec<u8> {
    let file_path = format!("src/tests/jobs/state_update_job/test_data/{}/{}", block_no, BLOB_DATA_FILE_NAME);
    let file_data = read_to_string(file_path).expect("Unable to read blob_data.txt").replace("0x", "");
    hex_string_to_u8_vec(&file_data).unwrap()
}

fn read_file_to_vec_u8_32(filename: &str) -> std::io::Result<Vec<[u8; 32]>> {
    let content = read_to_string(filename)?;
    let numbers: Vec<BigUint> = content.lines().filter_map(|line| line.parse().ok()).collect();

    Ok(numbers
        .into_iter()
        .map(|num| {
            let bytes = num.to_bytes_be();
            let mut array = [0u8; 32];
            let start = 32usize.saturating_sub(bytes.len());
            array[start..].copy_from_slice(&bytes);
            array
        })
        .collect())
}
