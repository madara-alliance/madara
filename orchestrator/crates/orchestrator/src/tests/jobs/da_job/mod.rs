use std::collections::HashMap;

use assert_matches::assert_matches;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use da_client_interface::MockDaClient;
use mockall::predicate::always;
use rstest::rstest;
use serde_json::json;
use starknet::core::types::{Felt, MaybePendingStateUpdate, PendingStateUpdate, StateDiff};
use uuid::Uuid;

use crate::jobs::da_job::test::{get_nonce_attached, read_state_update_from_file};
use crate::jobs::da_job::{DaError, DaJob};
use crate::jobs::types::{ExternalId, JobItem, JobStatus, JobType};
use crate::jobs::{Job, JobError};
use crate::tests::config::{ConfigType, TestConfigBuilder};

/// Tests the DA Job's handling of a blob length exceeding the supported size.
/// It mocks the DA client to simulate the environment and expects an error on job processing.
/// Validates the error message for exceeding blob limits against the expected output.
/// Asserts correct behavior by comparing the received and expected error messages.
#[rstest]
#[case(
    "src/tests/jobs/da_job/test_data/state_update/638353.txt",
    "src/tests/jobs/da_job/test_data/nonces/638353.txt",
    "63853",
    110
)]
#[tokio::test]
async fn test_da_job_process_job_failure_on_small_blob_size(
    #[case] state_update_file: String,
    #[case] nonces_file: String,
    #[case] internal_id: String,
    #[case] current_blob_length: u64,
) {
    // Mocking DA client calls
    let mut da_client = MockDaClient::new();
    // dummy state will have more than 1200 bytes
    da_client.expect_max_blob_per_txn().with().returning(|| 1);
    da_client.expect_max_bytes_per_blob().with().returning(|| 1200);

    let services = TestConfigBuilder::new()
        .configure_starknet_client(ConfigType::Actual)
        .configure_storage_client(ConfigType::Actual)
        .configure_da_client(da_client.into())
        .build()
        .await;

    let state_update = read_state_update_from_file(state_update_file.as_str()).expect("issue while reading");

    let state_update = MaybePendingStateUpdate::Update(state_update);
    let state_update = serde_json::to_value(&state_update).unwrap();
    let response = json!({ "id": 640641,"jsonrpc":"2.0","result": state_update });
    let server = services.starknet_server.unwrap();
    get_nonce_attached(&server, nonces_file.as_str());

    let state_update_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_getStateUpdate");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    let max_blob_per_txn = services.config.da_client().max_blob_per_txn().await;

    let response = DaJob
        .process_job(
            services.config,
            &mut JobItem {
                id: Uuid::default(),
                internal_id: internal_id.to_string(),
                job_type: JobType::DataSubmission,
                status: JobStatus::Created,
                external_id: ExternalId::String(internal_id.to_string().into_boxed_str()),
                metadata: HashMap::default(),
                version: 0,
                created_at: Utc::now().round_subsecs(0),
                updated_at: Utc::now().round_subsecs(0),
            },
        )
        .await;

    assert_matches!(response,
        Err(e) => {
            let err = DaError::MaxBlobsLimitExceeded { max_blob_per_txn, current_blob_length, block_no: internal_id.to_string(), job_id: Uuid::default() };
            let expected_error = JobError::DaJobError(err);
            assert_eq!(e.to_string(), expected_error.to_string());
        }
    );

    state_update_mock.assert();
    // let _ = drop_database().await;
}

/// Tests DA Job processing failure when a block is in pending state.
/// Simulates a pending block state update and expects job processing to fail.
/// Validates that the error message matches the expected pending state error.
/// Asserts correct behavior by comparing the received and expected error messages.
#[rstest]
#[tokio::test]
async fn test_da_job_process_job_failure_on_pending_block() {
    let services = TestConfigBuilder::new()
        .configure_starknet_client(ConfigType::Actual)
        .configure_da_client(ConfigType::Actual)
        .build()
        .await;
    let server = services.starknet_server.unwrap();
    let internal_id = "1";

    let pending_state_update = MaybePendingStateUpdate::PendingUpdate(PendingStateUpdate {
        old_root: Felt::default(),
        state_diff: StateDiff {
            storage_diffs: vec![],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
        },
    });

    let pending_state_update = serde_json::to_value(&pending_state_update).unwrap();
    let response = json!({ "id": 1,"jsonrpc":"2.0","result": pending_state_update });

    let state_update_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_getStateUpdate");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    let response = DaJob
        .process_job(
            services.config,
            &mut JobItem {
                id: Uuid::default(),
                internal_id: internal_id.to_string(),
                job_type: JobType::DataSubmission,
                status: JobStatus::Created,
                external_id: ExternalId::String("1".to_string().into_boxed_str()),
                metadata: HashMap::default(),
                version: 0,
                created_at: Utc::now().round_subsecs(0),
                updated_at: Utc::now().round_subsecs(0),
            },
        )
        .await;

    assert_matches!(response,
        Err(e) => {
            let err = DaError::BlockPending {
                block_no: internal_id.to_string(),
                job_id: Uuid::default()
            };
            let expected_error = JobError::DaJobError(err);
            assert_eq!(e.to_string(), expected_error.to_string());
        }
    );

    state_update_mock.assert();
}

/// Tests successful DA Job processing with valid state update and nonces files.
/// Mocks DA client to simulate environment and expects job to process without errors.
/// Validates the successful job processing by checking the return message "Done".
/// Asserts correct behavior by comparing the received and expected success messages.
#[rstest]
#[case(
    "src/tests/jobs/da_job/test_data/state_update/631861.txt",
    "src/tests/jobs/da_job/test_data/nonces/631861.txt",
    "631861"
)]
#[case(
    "src/tests/jobs/da_job/test_data/state_update/640641.txt",
    "src/tests/jobs/da_job/test_data/nonces/640641.txt",
    "640641"
)]
#[case(
    "src/tests/jobs/da_job/test_data/state_update/638353.txt",
    "src/tests/jobs/da_job/test_data/nonces/638353.txt",
    "638353"
)]
#[tokio::test]
async fn test_da_job_process_job_success(
    #[case] state_update_file: String,
    #[case] nonces_file: String,
    #[case] internal_id: String,
) {
    // Mocking DA client calls
    let mut da_client = MockDaClient::new();
    da_client.expect_publish_state_diff().with(always(), always()).returning(|_, _| Ok("Done".to_string()));
    da_client.expect_max_blob_per_txn().with().returning(|| 6);
    da_client.expect_max_bytes_per_blob().with().returning(|| 131072);

    let services = TestConfigBuilder::new()
        .configure_starknet_client(ConfigType::Actual)
        .configure_storage_client(ConfigType::Actual)
        .configure_da_client(da_client.into())
        .build()
        .await;
    let server = services.starknet_server.unwrap();

    let state_update = read_state_update_from_file(state_update_file.as_str()).expect("issue while reading");

    let state_update = serde_json::to_value(&state_update).unwrap();
    let response = json!({ "id": 1,"jsonrpc":"2.0","result": state_update });

    get_nonce_attached(&server, nonces_file.as_str());

    let state_update_mock = server.mock(|when, then| {
        when.path("/").body_includes("starknet_getStateUpdate");
        then.status(200).body(serde_json::to_vec(&response).unwrap());
    });

    let response = DaJob
        .process_job(
            services.config,
            &mut JobItem {
                id: Uuid::default(),
                internal_id: internal_id.to_string(),
                job_type: JobType::DataSubmission,
                status: JobStatus::Created,
                external_id: ExternalId::String(internal_id.to_string().into_boxed_str()),
                metadata: HashMap::default(),
                version: 0,
                created_at: Utc::now().round_subsecs(0),
                updated_at: Utc::now().round_subsecs(0),
            },
        )
        .await;

    assert_matches!(response,
        Ok(msg) => {
            assert_eq!(msg, eyre!("Done").to_string());
        }
    );

    state_update_mock.assert();
}
