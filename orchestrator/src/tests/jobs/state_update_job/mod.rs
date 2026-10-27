use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use crate::core::client::database::{AggregatorBatchDbQuery, MockDatabaseClient};
use crate::core::client::storage::MockStorageClient;
use crate::core::config::StarknetVersion;
use crate::error::job::state_update::StateUpdateError;
use crate::error::job::JobError;
use crate::tests::common::test_utils::{acquire_test_lock, get_job_handler_context_safe};
use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
use crate::types::batch::{AggregatorBatch, AggregatorBatchWeights};
use crate::types::constant::{
    get_batch_artifact_file, BLOB_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
};
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, SettlementContext, SettlementContextData, StateUpdateMetadata,
};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::jobs::state_update::StateUpdateJobHandler;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use crate::worker::event_handler::service::JobHandlerService;
use assert_matches::assert_matches;
use async_trait::async_trait;
use bytes::Bytes;
use httpmock::prelude::*;
use lazy_static::lazy_static;
use mockall::predicate::eq;
use orchestrator_settlement_client_interface::{
    MockSettlementClient, SettlementClient, SettlementVerificationStatus, StateUpdateTxResult,
};
use rstest::*;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use url::Url;

lazy_static! {
    pub static ref CURRENT_PATH: PathBuf = std::env::current_dir().expect("Failed to get Current Path");
}

pub const X_0_FILE_NAME: &str = "x_0.txt";

struct PendingFirstStateUpdateClient {
    broadcast_tx: Mutex<Option<oneshot::Sender<()>>>,
    submissions: Arc<AtomicUsize>,
    submitted_nonces: Arc<Mutex<Vec<u64>>>,
    last_settled_block: u64,
    nonce: u64,
}

#[async_trait]
impl SettlementClient for PendingFirstStateUpdateClient {
    async fn register_proof(&self, _proof: [u8; 32]) -> color_eyre::Result<String> {
        unreachable!("register_proof is not used by this test")
    }

    async fn update_state_calldata(
        &self,
        _snos_output: Vec<[u8; 32]>,
        _program_output: Vec<[u8; 32]>,
        _onchain_data_hash: [u8; 32],
        _onchain_data_size: [u8; 32],
        _nonce: u64,
    ) -> color_eyre::Result<String> {
        unreachable!("update_state_calldata is not used by this test")
    }

    async fn update_state_with_blobs(
        &self,
        _program_output: Vec<[u8; 32]>,
        _state_diff: Vec<Vec<u8>>,
        nonce: u64,
    ) -> color_eyre::Result<StateUpdateTxResult> {
        self.submitted_nonces.lock().unwrap().push(nonce);

        if self.submissions.fetch_add(1, Ordering::SeqCst) == 0 {
            if let Some(broadcast_tx) = self.broadcast_tx.lock().unwrap().take() {
                let _ = broadcast_tx.send(());
            }
            std::future::pending().await
        } else {
            Ok(StateUpdateTxResult { tx_hash: "0xreplacement".to_string(), attempts: Vec::new() })
        }
    }

    async fn verify_tx_inclusion(&self, _tx_hash: &str) -> color_eyre::Result<SettlementVerificationStatus> {
        unreachable!("verify_tx_inclusion is not used by this test")
    }

    async fn wait_for_tx_finality(&self, _tx_hash: &str) -> color_eyre::Result<Option<u64>> {
        unreachable!("wait_for_tx_finality is not used by this test")
    }

    async fn get_last_settled_block(&self) -> color_eyre::Result<Option<u64>> {
        Ok(Some(self.last_settled_block))
    }

    async fn get_nonce(&self) -> color_eyre::Result<u64> {
        Ok(self.nonce + self.submissions.load(Ordering::SeqCst) as u64)
    }
}

// ==================== Mock Tests (Unit tests) ===========================

#[rstest]
#[tokio::test]
async fn create_job_works() {
    // Create proper metadata structure
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_path: Some(format!("1/{}", SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("1/{}", PROGRAM_OUTPUT_FILE_NAME)),
            blob_data_path: Some(format!("1/{}", BLOB_DATA_FILE_NAME)),
            da_segment_path: None,
            tx_hash: None,
            tx_nonce: None,
            inflight_tx_nonce: None,
            tx_attempts: Vec::new(),
            context: SettlementContext::Block(SettlementContextData { to_settle: 1, last_failed: None }),
            storage_artifacts_tagged_at: None,
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
            snos_output_path: Some(format!("{}/{}", 6, SNOS_OUTPUT_FILE_NAME)),
            program_output_path: Some(format!("{}/{}", 6, PROGRAM_OUTPUT_FILE_NAME)),
            blob_data_path: Some(format!("{}/{}", 6, BLOB_DATA_FILE_NAME)),
            da_segment_path: None,
            tx_hash: None,
            tx_nonce: None,
            inflight_tx_nonce: None,
            tx_attempts: Vec::new(),
            context: SettlementContext::Block(SettlementContextData {
                to_settle: 6, // Gap between 4 and 6
                last_failed: None,
            }),
            storage_artifacts_tagged_at: None,
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

#[rstest]
#[tokio::test]
async fn verify_job_waits_when_contract_is_at_previous_batch_boundary() {
    let current_batch_index = 559;
    let previous_batch_index = 558;
    let previous_end_block = 902785;
    let current_end_block = 902875;

    let mut settlement_client = MockSettlementClient::new();
    settlement_client.expect_get_last_settled_block().times(1).returning(move || Ok(Some(previous_end_block)));

    let mut database_client = MockDatabaseClient::new();
    database_client.expect_get_aggregator_batches().times(2).returning(
        move |query: AggregatorBatchDbQuery| match query.indexes.as_deref() {
            Some([index]) if *index == current_batch_index => {
                Ok(vec![test_aggregator_batch(current_batch_index, previous_end_block + 1, current_end_block)])
            }
            Some([index]) if *index == previous_batch_index => {
                Ok(vec![test_aggregator_batch(previous_batch_index, 902695, previous_end_block)])
            }
            _ => panic!("unexpected aggregator batch query: {:?}", query.indexes),
        },
    );

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    let mut job = state_transition_job_for_batch(current_batch_index).await;
    let result = StateUpdateJobHandler.verify_job(services.config, &mut job).await;

    assert_eq!(result.unwrap(), JobVerificationStatus::Pending);
}

#[rstest]
#[tokio::test]
async fn verify_job_rejects_when_contract_is_behind_previous_batch_boundary() {
    let current_batch_index = 559;
    let previous_batch_index = 558;
    let previous_end_block = 902785;
    let current_end_block = 902875;
    let actual_settled_block = 902700;

    let mut settlement_client = MockSettlementClient::new();
    settlement_client.expect_get_last_settled_block().times(1).returning(move || Ok(Some(actual_settled_block)));

    let mut database_client = MockDatabaseClient::new();
    database_client.expect_get_aggregator_batches().times(2).returning(
        move |query: AggregatorBatchDbQuery| match query.indexes.as_deref() {
            Some([index]) if *index == current_batch_index => {
                Ok(vec![test_aggregator_batch(current_batch_index, previous_end_block + 1, current_end_block)])
            }
            Some([index]) if *index == previous_batch_index => {
                Ok(vec![test_aggregator_batch(previous_batch_index, 902695, previous_end_block)])
            }
            _ => panic!("unexpected aggregator batch query: {:?}", query.indexes),
        },
    );

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    let mut job = state_transition_job_for_batch(current_batch_index).await;
    let result = StateUpdateJobHandler.verify_job(services.config, &mut job).await;

    assert_eq!(
        result.unwrap(),
        JobVerificationStatus::Rejected(format!(
            "Last settle block expected was {} but found {}",
            current_end_block, actual_settled_block
        ))
    );
}

#[rstest]
#[tokio::test]
async fn settled_batch_reconciliation_skips_state_update_submission() {
    let batch_index = 559;
    let end_block = 902875;

    let mut settlement_client = MockSettlementClient::new();
    settlement_client.expect_get_last_settled_block().times(1).returning(move || Ok(Some(end_block)));

    let mut database_client = MockDatabaseClient::new();
    database_client.expect_get_aggregator_batches().times(1).returning(move |query: AggregatorBatchDbQuery| {
        assert_eq!(query.indexes, Some(vec![batch_index]));
        Ok(vec![test_aggregator_batch(batch_index, 902786, end_block)])
    });

    let services = TestConfigBuilder::new()
        .configure_settlement_client(settlement_client.into())
        .configure_database(database_client.into())
        .build()
        .await;

    let should_submit =
        StateUpdateJobHandler.should_send_state_update_txn_from_chain(&services.config, batch_index).await;

    assert!(!should_submit.unwrap(), "an already-settled batch must not be submitted again");
}

/// A shutdown after broadcast preserves the pinned nonce even if the provider's next nonce has
/// advanced. The retry replaces the same transaction rather than creating a second transition.
#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn shutdown_after_state_update_broadcast_retries_with_the_same_nonce() {
    let _test_lock = acquire_test_lock();
    let batch_index = 559;
    let nonce = 42;
    let program_output_path = format!("batch/{batch_index}/{PROGRAM_OUTPUT_FILE_NAME}");
    let da_segment_path = format!("batch/{batch_index}/da_segment.json");

    let (broadcast_tx, broadcast_rx) = oneshot::channel();
    let submissions = Arc::new(AtomicUsize::new(0));
    let submitted_nonces = Arc::new(Mutex::new(Vec::new()));

    let settlement_client = PendingFirstStateUpdateClient {
        broadcast_tx: Mutex::new(Some(broadcast_tx)),
        submissions: Arc::clone(&submissions),
        submitted_nonces: Arc::clone(&submitted_nonces),
        last_settled_block: 902785,
        nonce,
    };

    let serialized_program_output = bincode::serialize(&vec![[0_u8; 32]]).unwrap();
    let mut storage_client = MockStorageClient::new();
    storage_client
        .expect_get_data()
        .with(eq(program_output_path.clone()))
        .times(2)
        .returning(move |_| Ok(Bytes::from(serialized_program_output.clone())));
    storage_client
        .expect_get_data()
        .with(eq(da_segment_path.clone()))
        .times(2)
        .returning(|_| Ok(Bytes::from_static(br#"["0x1"]"#)));

    let services = TestConfigBuilder::new()
        .configure_database(ConfigType::Actual)
        .configure_queue_client(ConfigType::Actual)
        .configure_storage_client(ConfigType::Mock(MockType::Storage(Box::new(storage_client))))
        .configure_settlement_client(ConfigType::Mock(MockType::SettlementClient(Box::new(settlement_client))))
        .build()
        .await;
    services
        .config
        .database()
        .create_aggregator_batch(test_aggregator_batch(batch_index, 902786, 902875))
        .await
        .unwrap();

    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_path: None,
            program_output_path: Some(program_output_path),
            blob_data_path: None,
            da_segment_path: Some(da_segment_path),
            tx_hash: None,
            tx_nonce: None,
            inflight_tx_nonce: None,
            tx_attempts: Vec::new(),
            context: SettlementContext::Batch(SettlementContextData { to_settle: batch_index, last_failed: None }),
            storage_artifacts_tagged_at: None,
        }),
    };
    let job = StateUpdateJobHandler.create_job(batch_index, metadata).await.unwrap();
    services.config.database().create_job(job.clone()).await.unwrap();

    let handler: Arc<Box<dyn JobHandlerTrait>> = Arc::new(Box::new(StateUpdateJobHandler));
    let ctx = get_job_handler_context_safe();
    ctx.expect().times(2).with(eq(JobType::StateTransition)).returning(move |_| Arc::clone(&handler));

    let shutdown_token = CancellationToken::new();
    let first_attempt = tokio::spawn(JobHandlerService::process_job_with_shutdown(
        job.id,
        services.config.clone(),
        shutdown_token.clone(),
    ));
    broadcast_rx.await.expect("first state update transaction should be broadcast");
    shutdown_token.cancel();

    let (mut locked_job, original_status, original_metadata) = match first_attempt.await.unwrap() {
        Err(JobError::ShutdownCleanupPending { id, job: locked_job, original_status, original_metadata }) => {
            assert_eq!(id, job.id);
            (*locked_job, original_status, *original_metadata)
        }
        other => panic!("expected shutdown cleanup, got {other:?}"),
    };
    let locked_metadata: StateUpdateMetadata = locked_job.metadata.specific.clone().try_into().unwrap();
    assert_eq!(locked_metadata.inflight_tx_nonce, Some(nonce));
    assert_matches!(
        JobHandlerService::restore_job_after_shutdown(
            &mut locked_job,
            services.config.clone(),
            original_status,
            original_metadata,
        )
        .await,
        JobError::ShutdownInterrupted { .. }
    );
    let restored_job = services.config.database().get_job_by_id(job.id).await.unwrap().unwrap();
    assert_eq!(restored_job.status, JobStatus::Created);
    let restored_metadata: StateUpdateMetadata = restored_job.metadata.specific.clone().try_into().unwrap();
    assert_eq!(restored_metadata.inflight_tx_nonce, Some(nonce));

    JobHandlerService::process_job(job.id, services.config.clone()).await.unwrap();

    let retried_job = services.config.database().get_job_by_id(job.id).await.unwrap().unwrap();
    let retried_metadata: StateUpdateMetadata = retried_job.metadata.specific.try_into().unwrap();
    assert_eq!(retried_job.status, JobStatus::PendingVerification);
    assert_eq!(retried_metadata.tx_hash.as_deref(), Some("0xreplacement"));
    assert_eq!(retried_metadata.tx_nonce, Some(nonce));
    assert_eq!(retried_metadata.inflight_tx_nonce, None);
    assert_eq!(*submitted_nonces.lock().unwrap(), vec![nonce, nonce]);
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
    database_client.expect_get_aggregator_batches().returning(move |query: AggregatorBatchDbQuery| {
        assert_eq!(query.indexes, Some(vec![batch_index]));
        Ok(vec![AggregatorBatch::new(
            batch_index,
            end_block + 1,
            Some(String::from("")),
            256,
            1,
            AggregatorBatchWeights::default(),
            StarknetVersion::V0_14_1,
        )])
    });
    database_client.expect_update_job().times(1).returning(|job, updates| {
        let mut updated = job.clone();
        updated.metadata = updates.metadata.expect("nonce intent must be persisted before submission");
        updated.version += 1;
        Ok(updated)
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
        .returning(|_, _, _| Ok(StateUpdateTxResult { tx_hash: "0xabcd".to_string(), attempts: Vec::new() }));

    // Mock storage for DA segment
    let da_segment_key = get_batch_artifact_file(batch_index, DA_SEGMENT_FILE_NAME);
    let da_json_clone = da_json.clone();
    storage_client
        .expect_get_data()
        .with(eq(da_segment_key.clone()))
        .returning(move |_| Ok(Bytes::from(da_json_clone.clone())));

    // Mock storage for program output
    let program_output_key = get_batch_artifact_file(batch_index, PROGRAM_OUTPUT_FILE_NAME);
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
            snos_output_path: None, // Not used for L2 batch settlement
            program_output_path: Some(program_output_key),
            blob_data_path: None, // Not used for L2 with DA segments
            da_segment_path: Some(da_segment_key),
            tx_hash: None,
            tx_nonce: None,
            inflight_tx_nonce: None,
            tx_attempts: Vec::new(),
            context: SettlementContext::Batch(SettlementContextData { to_settle: batch_index, last_failed: None }),
            storage_artifacts_tagged_at: None,
        }),
    };

    let mut job = StateUpdateJobHandler.create_job(0, metadata).await.unwrap();
    let result = StateUpdateJobHandler.process_job(services.config, &mut job).await;

    assert!(result.is_ok(), "L2 state update with DA segment should succeed: {:?}", result.err());
}

// ==================== Utility functions ===========================

fn test_aggregator_batch(index: u64, start_block: u64, end_block: u64) -> AggregatorBatch {
    AggregatorBatch::new(
        index,
        start_block,
        Some(String::new()),
        256,
        1,
        AggregatorBatchWeights::default(),
        StarknetVersion::V0_14_1,
    )
    .update(end_block, 256, 1, AggregatorBatchWeights::default(), None)
}

async fn state_transition_job_for_batch(batch_index: u64) -> crate::types::jobs::job_item::JobItem {
    let metadata = JobMetadata {
        common: CommonMetadata::default(),
        specific: JobSpecificMetadata::StateUpdate(StateUpdateMetadata {
            snos_output_path: None,
            program_output_path: Some(format!("batch/{batch_index}/program_output.txt")),
            blob_data_path: None,
            da_segment_path: Some(format!("batch/{batch_index}/da_blob.json")),
            tx_hash: Some("0x1234".to_string()),
            tx_nonce: None,
            inflight_tx_nonce: None,
            tx_attempts: Vec::new(),
            context: SettlementContext::Batch(SettlementContextData { to_settle: batch_index, last_failed: None }),
            storage_artifacts_tagged_at: None,
        }),
    };

    StateUpdateJobHandler.create_job(batch_index, metadata).await.expect("state transition job should be created")
}
