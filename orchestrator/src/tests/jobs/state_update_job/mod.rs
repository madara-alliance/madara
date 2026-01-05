use std::fs::read_to_string;
use std::path::PathBuf;

use crate::core::client::database::MockDatabaseClient;
use crate::core::client::storage::MockStorageClient;
use crate::core::config::StarknetVersion;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::tests::common::default_job_item;
use crate::tests::config::TestConfigBuilder;
use crate::types::batch::{AggregatorBatch, AggregatorBatchWeights};
use crate::types::constant::{
    BLOB_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME, STORAGE_ARTIFACTS_DIR,
};
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::state_update::StateUpdateJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use assert_matches::assert_matches;
use bytes::Bytes;
use httpmock::prelude::*;
use lazy_static::lazy_static;
use mockall::predicate::eq;
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
        da_segment_paths: vec![],
        tx_hash: None,
        context: SettlementContext::Block(SettlementContextData { to_settle: vec![], last_failed: None }),
    });

    let res = StateUpdateJobHandler.process_job(services.config, &mut job).await.unwrap_err();
    assert!(
        matches!(res, JobError::StateUpdateJobError(StateUpdateError::BlockNumberNotFound)),
        "JobError should be StateUpdateJobError with BlockNumberNotFound"
    );
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
            da_segment_paths: vec![],
            tx_hash: None,
            context: SettlementContext::Block(SettlementContextData { to_settle: vec![1], last_failed: None }),
        }),
    };

    let job = StateUpdateJobHandler.create_job(0, metadata).await;
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
            da_segment_paths: vec![],
            tx_hash: None,
            context: SettlementContext::Block(SettlementContextData { to_settle: block_numbers, last_failed: None }),
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(0, metadata).await.unwrap();
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
            da_segment_paths: vec![],
            tx_hash: None,
            context: SettlementContext::Block(SettlementContextData {
                to_settle: vec![6, 7, 8], // Gap between 4 and 6
                last_failed: None,
            }),
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(0, metadata).await.unwrap();
    let response = StateUpdateJobHandler.process_job(services.config, &mut job).await;

    assert_matches!(response,
        Err(e) => {
            let err = StateUpdateError::GapBetweenFirstAndLastBlock;
            let expected_error = JobError::StateUpdateJobError(err);
            assert_eq!(e.to_string(), expected_error.to_string());
        }
    );
}

// ==================== L2 DA Segment Tests ===========================

/// Test L2 state update flow with encrypted DA segments from prover.
/// Uses Paradex testnet batch data (blocks 490000-490003).
#[rstest]
#[case(1, 490000, 490001)] // Batch 1
#[case(2, 490002, 490003)] // Batch 2
#[tokio::test]
async fn test_process_job_l2_with_da_segment(
    #[case] batch_index: u64,
    #[case] start_block: u64,
    #[case] end_block: u64,
) {
    use crate::types::constant::DA_SEGMENT_FILE_NAME;
    use crate::worker::utils::encrypted_blob::{da_segment_to_blobs, parse_da_segment_json};
    use cairo_vm::Felt252;
    use orchestrator_utils::test_utils::setup_test_data;

    dotenvy::from_filename_override("../.env.test").expect("Failed to load .env.test file");

    // Download test artifacts from remote repository
    let da_segment_file = format!("da_blob_index_{}.json", batch_index);
    let program_output_file = format!("program_output_batch_{}.json", batch_index);
    let data_dir = setup_test_data(vec![
        (Box::leak(da_segment_file.clone().into_boxed_str()) as &str, false),
        (Box::leak(program_output_file.clone().into_boxed_str()) as &str, false),
    ])
    .await
    .expect("Failed to download test artifacts");

    // Load DA segment and convert to blobs (this is what the state update job does)
    let da_json = read_to_string(data_dir.path().join(&da_segment_file)).expect("Failed to read DA segment");
    let da_segment = parse_da_segment_json(&da_json).expect("Failed to parse DA segment");
    let blobs = da_segment_to_blobs(da_segment).expect("Failed to convert to blobs");

    // Load program output
    let program_output_json =
        read_to_string(data_dir.path().join(&program_output_file)).expect("Failed to read program output");
    let program_output_hex: Vec<String> = serde_json::from_str(&program_output_json).unwrap();
    let program_output: Vec<[u8; 32]> = program_output_hex
        .iter()
        .map(|hex| {
            let felt = Felt252::from_hex(hex).unwrap();
            felt.to_bytes_be()
        })
        .collect();

    // Mock clients
    let mut settlement_client = MockSettlementClient::new();
    let mut storage_client = MockStorageClient::new();
    let mut database_client = MockDatabaseClient::new();

    // Mock database batch lookup
    database_client.expect_get_aggregator_batches_by_indexes().returning(move |_| {
        Ok(vec![AggregatorBatch::new(
            batch_index,
            end_block + 1,
            String::from(""),
            256,
            AggregatorBatchWeights::default(),
            StarknetVersion::V0_14_1,
        )])
    });

    // Mock settlement client
    settlement_client.expect_get_last_settled_block().returning(move || Ok(Some(start_block - 1)));
    settlement_client.expect_get_nonce().returning(|| Ok(1));

    // Expect update_state_with_blobs to be called with correct data
    let expected_program_output = program_output.clone();
    let expected_blobs = blobs.clone();
    settlement_client
        .expect_update_state_with_blobs()
        .withf(move |po, blobs, _| po == &expected_program_output && blobs == &expected_blobs)
        .times(1)
        .returning(|_, _, _| Ok("0xabcd".to_string()));

    settlement_client.expect_wait_for_tx_finality().with(eq("0xabcd")).times(1).returning(|_| Ok(Some(1)));

    // Mock storage for DA segment
    let da_segment_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, batch_index, DA_SEGMENT_FILE_NAME);
    let da_json_clone = da_json.clone();
    storage_client
        .expect_get_data()
        .with(eq(da_segment_key.clone()))
        .returning(move |_| Ok(Bytes::from(da_json_clone.clone())));

    // Mock storage for program output
    let program_output_key = format!("{}/batch/{}/{}", STORAGE_ARTIFACTS_DIR, batch_index, PROGRAM_OUTPUT_FILE_NAME);
    let serialized_program_output = bincode::serialize(&program_output).unwrap();
    storage_client
        .expect_get_data()
        .with(eq(program_output_key.clone()))
        .returning(move |_| Ok(Bytes::from(serialized_program_output.clone())));

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_storage_client(storage_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    // Create job with L2 DA segment paths
    let metadata = JobMetadata {
        common: CommonMetadata { process_attempt_no: 0, ..CommonMetadata::default() },
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_paths: vec![], // Not used for L2 batch settlement
            program_output_paths: vec![program_output_key],
            blob_data_paths: vec![], // Not used for L2 with DA segments
            da_segment_paths: vec![da_segment_key],
            tx_hash: None,
            context: SettlementContext::Batch(SettlementContextData {
                to_settle: vec![batch_index],
                last_failed: None,
            }),
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(0, metadata).await.unwrap();
    let result = StateUpdateJobHandler.process_job(services.config, &mut job).await;

    assert!(result.is_ok(), "L2 state update with DA segment should succeed: {:?}", result.err());
}

// ==================== Utility functions ===========================
