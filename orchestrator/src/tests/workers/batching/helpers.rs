use crate::core::client::database::MockDatabaseClient;
use crate::core::client::lock::{LockResult, LockValue, MockLockClient};
use crate::core::client::storage::MockStorageClient;
use crate::core::config::StarknetVersion;
use crate::tests::config::TestConfigBuilder;
use crate::types::batch::{
    AggregatorBatch, AggregatorBatchStatus, AggregatorBatchWeights, SnosBatch, SnosBatchStatus,
};
use blockifier::bouncer::BouncerWeights;
use bytes::Bytes;
use httpmock::MockServer;
use orchestrator_prover_client_interface::MockProverClient;
use serde_json::json;
use starknet_api::execution_resources::GasAmount;
use starknet_core::types::{Felt, MaybePreConfirmedStateUpdate, StateDiff, StateUpdate};
use std::ops::Range;
use std::sync::Arc;

/// Test configuration for batching tests
pub struct BatchingTestSetup {
    pub server: MockServer,
    pub database: MockDatabaseClient,
    pub storage: MockStorageClient,
    pub lock: MockLockClient,
    pub prover: MockProverClient,
}

impl BatchingTestSetup {
    pub fn new() -> Self {
        Self {
            server: MockServer::start(),
            database: MockDatabaseClient::new(),
            storage: MockStorageClient::new(),
            lock: MockLockClient::new(),
            prover: MockProverClient::new(),
        }
    }

    /// Configure common lock expectations for batching workers
    pub fn setup_lock_expectations(&mut self) {
        self.lock
            .expect_acquire_lock()
            .withf(|key, value, expiry_seconds, owner| {
                (key == "AggregatorBatchingWorker" || key == "SnosBatchingWorker")
                    && *value == LockValue::Boolean(false)
                    && *expiry_seconds == 3600
                    && owner.is_none()
            })
            .returning(|_, _, _, _| Ok(LockResult::Acquired));

        self.lock
            .expect_release_lock()
            .withf(|key, owner| {
                (key == "AggregatorBatchingWorker" || key == "SnosBatchingWorker") && owner.is_none()
            })
            .returning(|_, _| Ok(LockResult::Released));
    }

    /// Setup storage expectations for reading and writing data
    pub fn setup_storage_expectations(&mut self, read_state_update: Option<StateUpdate>) {
        if let Some(state_update) = read_state_update {
            self.storage
                .expect_get_data()
                .returning(move |_| Ok(Bytes::from(serde_json::to_string(&state_update).unwrap())));
        }
        self.storage.expect_put_data().returning(|_, _| Ok(()));
    }

    /// Setup basic aggregator database expectations
    pub fn setup_aggregator_db_expectations(&mut self, existing_batch: Option<AggregatorBatch>) {
        let batch_clone = existing_batch.clone();
        self.database
            .expect_get_latest_aggregator_batch()
            .returning(move || Ok(batch_clone.clone()));

        self.database
            .expect_create_aggregator_batch()
            .returning(Ok);
        self.database
            .expect_update_or_create_aggregator_batch()
            .returning(|batch, _| Ok(batch.clone()));
    }

    /// Setup basic SNOS database expectations
    pub fn setup_snos_db_expectations(&mut self, existing_batch: Option<SnosBatch>) {
        let batch_clone = existing_batch.clone();
        self.database
            .expect_get_latest_snos_batch()
            .returning(move || Ok(batch_clone.clone()));

        self.database
            .expect_create_snos_batch()
            .returning(Ok);
        self.database
            .expect_update_or_create_snos_batch()
            .returning(|batch, _| Ok(batch.clone()));
        self.database.expect_get_next_snos_batch_id().returning(|| Ok(1));
        self.database
            .expect_get_open_snos_batches_by_aggregator_index()
            .returning(|_| Ok(vec![]));
        self.database
            .expect_close_all_snos_batches_for_aggregator()
            .returning(|_| Ok(vec![]));
    }

    /// Setup HTTP mocks for RPC calls for a range of blocks
    pub fn setup_block_mocks(&self, block_range: Range<u64>, version: &str) {
        let end_block = block_range.end - 1;

        // Mock starknet_blockNumber
        self.server.mock(|when, then| {
            when.path("/").body_includes("starknet_blockNumber");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": end_block })).unwrap());
        });

        // Mock starknet_getBlockWithTxHashes and starknet_getStateUpdate for each block
        for block_num in block_range.clone() {
            self.setup_single_block_mock(block_num, version);
        }

        // Mock bouncer weights
        self.setup_bouncer_weights_mock();
    }

    /// Setup HTTP mock for a single block
    pub fn setup_single_block_mock(&self, block_num: u64, version: &str) {
        // Mock starknet_getBlockWithTxHashes
        let pattern = format!(r#".*"block_number"\s*:\s*{}[,\}}].*"#, block_num);
        self.server.mock(move |when, then| {
            when.path("/")
                .body_includes("starknet_getBlockWithTxHashes")
                .body_matches(pattern.as_str());
            then.status(200).body(
                serde_json::to_vec(&json!({
                    "jsonrpc": "2.0",
                    "result": generate_block_with_tx_hashes(block_num, version),
                    "id": 1
                }))
                .unwrap(),
            );
        });

        // Mock starknet_getStateUpdate
        let state_update = generate_state_update(block_num, 0);
        self.server.mock(|when, then| {
            when.path("/").body_includes("starknet_getStateUpdate");
            then.status(200)
                .body(serde_json::to_vec(&json!({ "id": 1, "jsonrpc": "2.0", "result": state_update })).unwrap());
        });
    }

    /// Setup bouncer weights mock
    pub fn setup_bouncer_weights_mock(&self) {
        let builtin_weights = generate_bouncer_weights();
        self.server.mock(|when, then| {
            when.path("/feeder_gateway/get_block_bouncer_weights");
            then.status(200).body(serde_json::to_vec(&builtin_weights).unwrap());
        });
    }

    /// Build the test configuration
    pub async fn build(self) -> Arc<crate::core::config::Config> {
        let provider_url = format!("http://localhost:{}", self.server.port());
        let provider = starknet::providers::JsonRpcClient::new(starknet::providers::jsonrpc::HttpTransport::new(
            url::Url::parse(&provider_url).expect("Failed to parse URL"),
        ));

        TestConfigBuilder::new()
            .configure_starknet_client(provider.into())
            .configure_madara_feeder_gateway_url(&provider_url)
            .configure_prover_client(self.prover.into())
            .configure_storage_client(self.storage.into())
            .configure_database(self.database.into())
            .configure_lock_client(self.lock.into())
            .build()
            .await
            .config
    }

    /// Build config with a specific layer
    pub async fn build_with_layer(self, layer: orchestrator_utils::layer::Layer) -> Arc<crate::core::config::Config> {
        let provider_url = format!("http://localhost:{}", self.server.port());
        let provider = starknet::providers::JsonRpcClient::new(starknet::providers::jsonrpc::HttpTransport::new(
            url::Url::parse(&provider_url).expect("Failed to parse URL"),
        ));

        TestConfigBuilder::new()
            .configure_starknet_client(provider.into())
            .configure_madara_feeder_gateway_url(&provider_url)
            .configure_prover_client(self.prover.into())
            .configure_storage_client(self.storage.into())
            .configure_database(self.database.into())
            .configure_lock_client(self.lock.into())
            .configure_layer(layer)
            .build()
            .await
            .config
    }

    /// Get the provider URL
    #[allow(dead_code)]
    pub fn provider_url(&self) -> String {
        format!("http://localhost:{}", self.server.port())
    }
}

impl Default for BatchingTestSetup {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Data Generators
// ============================================================================

/// Generate a dummy block with transaction hashes
pub fn generate_block_with_tx_hashes(block_num: u64, starknet_version: &str) -> serde_json::Value {
    json!({
        "status": "ACCEPTED_ON_L1",
        "block_hash": format!("0x{:x}", block_num),
        "parent_hash": format!("0x{:x}", block_num.saturating_sub(1)),
        "block_number": block_num,
        "new_root": format!("0x{:x}", block_num + 1),
        "timestamp": 1234567890 + block_num,
        "sequencer_address": "0x0",
        "l1_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l2_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l1_data_gas_price": {
            "price_in_fri": "0x1",
            "price_in_wei": "0x1"
        },
        "l1_da_mode": "CALLDATA",
        "starknet_version": starknet_version,
        "transactions": []
    })
}

/// Generate a state update for testing
pub fn generate_state_update(block_num: u64, _size: usize) -> serde_json::Value {
    let state_update = MaybePreConfirmedStateUpdate::Update(StateUpdate {
        block_hash: Felt::from(block_num),
        new_root: Felt::from(block_num + 1),
        old_root: Felt::from(block_num),
        state_diff: StateDiff {
            storage_diffs: vec![],
            deprecated_declared_classes: vec![],
            declared_classes: vec![],
            deployed_contracts: vec![],
            replaced_classes: vec![],
            nonces: vec![],
        },
    });

    serde_json::to_value(&state_update).unwrap()
}

/// Generate bouncer weights for testing
pub fn generate_bouncer_weights() -> BouncerWeights {
    BouncerWeights {
        l1_gas: 500_000,
        message_segment_length: 700,
        n_events: 1000,
        n_txs: 100,
        state_diff_size: 1000,
        sierra_gas: GasAmount(1_000_000_000),
        proving_gas: GasAmount(1_100_000_000),
    }
}

/// Generate custom bouncer weights
pub fn generate_custom_bouncer_weights(
    l1_gas: usize,
    message_segment_length: usize,
    n_events: usize,
    n_txs: usize,
    state_diff_size: usize,
    sierra_gas: u64,
    proving_gas: u64,
) -> BouncerWeights {
    BouncerWeights {
        l1_gas,
        message_segment_length,
        n_events,
        n_txs,
        state_diff_size,
        sierra_gas: GasAmount(sierra_gas),
        proving_gas: GasAmount(proving_gas),
    }
}

/// Generate custom aggregator batch weights
pub fn generate_custom_aggregator_weights(l1_gas: usize, message_segment_length: usize) -> AggregatorBatchWeights {
    AggregatorBatchWeights { l1_gas, message_segment_length }
}

// ============================================================================
// Assertion Helpers
// ============================================================================

/// Validate that batches have no gaps in block ranges
pub fn assert_no_gaps(batches: &[AggregatorBatch]) {
    for i in 0..batches.len().saturating_sub(1) {
        let current = &batches[i];
        let next = &batches[i + 1];
        assert_eq!(
            current.end_block + 1,
            next.start_block,
            "Gap detected between batch {} (ending at {}) and batch {} (starting at {})",
            current.index,
            current.end_block,
            next.index,
            next.start_block
        );
    }
}

/// Validate that batches have no overlaps in block ranges
pub fn assert_no_overlaps(batches: &[AggregatorBatch]) {
    for i in 0..batches.len().saturating_sub(1) {
        let current = &batches[i];
        let next = &batches[i + 1];
        assert!(
            current.end_block < next.start_block,
            "Overlap detected: batch {} ends at {} but batch {} starts at {}",
            current.index,
            current.end_block,
            next.index,
            next.start_block
        );
    }
}

/// Validate batch indices are sequential
#[allow(dead_code)]
pub fn assert_sequential_indices(batches: &[AggregatorBatch]) {
    for i in 0..batches.len().saturating_sub(1) {
        let current = &batches[i];
        let next = &batches[i + 1];
        assert_eq!(
            current.index + 1,
            next.index,
            "Non-sequential indices: batch {} followed by batch {}",
            current.index,
            next.index
        );
    }
}

/// Validate that all blocks in a range are covered by batches
#[allow(dead_code)]
pub fn assert_complete_coverage(batches: &[AggregatorBatch], expected_start: u64, expected_end: u64) {
    if batches.is_empty() {
        panic!("No batches provided for coverage validation");
    }

    let first_batch = batches.first().unwrap();
    let last_batch = batches.last().unwrap();

    assert_eq!(
        first_batch.start_block, expected_start,
        "First batch should start at {}, but starts at {}",
        expected_start, first_batch.start_block
    );

    assert_eq!(
        last_batch.end_block, expected_end,
        "Last batch should end at {}, but ends at {}",
        expected_end, last_batch.end_block
    );

    let total_blocks: u64 = batches.iter().map(|b| b.num_blocks).sum();
    let expected_total = expected_end - expected_start + 1;
    assert_eq!(
        total_blocks, expected_total,
        "Total blocks covered ({}) doesn't match expected range ({} to {} = {} blocks)",
        total_blocks, expected_start, expected_end, expected_total
    );
}

/// Validate aggregator batch properties
pub fn assert_aggregator_batch_properties(
    batch: &AggregatorBatch,
    expected_index: u64,
    expected_start: u64,
    expected_end: u64,
    expected_status: AggregatorBatchStatus,
    expected_version: StarknetVersion,
) {
    assert_eq!(batch.index, expected_index, "Batch index mismatch");
    assert_eq!(batch.start_block, expected_start, "Batch start_block mismatch");
    assert_eq!(batch.end_block, expected_end, "Batch end_block mismatch");
    assert_eq!(
        batch.num_blocks,
        expected_end - expected_start + 1,
        "Batch num_blocks doesn't match block range"
    );
    assert_eq!(batch.status, expected_status, "Batch status mismatch");
    assert_eq!(batch.starknet_version, expected_version, "Batch starknet_version mismatch");
}

/// Validate SNOS batch properties
#[allow(dead_code)]
pub fn assert_snos_batch_properties(
    batch: &SnosBatch,
    expected_index: u64,
    expected_aggregator_index: Option<u64>,
    expected_start: u64,
    expected_end: u64,
    expected_status: SnosBatchStatus,
    expected_version: StarknetVersion,
) {
    assert_eq!(batch.index, expected_index, "SNOS batch index mismatch");
    assert_eq!(
        batch.aggregator_batch_index, expected_aggregator_index,
        "SNOS batch aggregator_batch_index mismatch"
    );
    assert_eq!(batch.start_block, expected_start, "SNOS batch start_block mismatch");
    assert_eq!(batch.end_block, expected_end, "SNOS batch end_block mismatch");
    assert_eq!(
        batch.num_blocks,
        expected_end - expected_start + 1,
        "SNOS batch num_blocks doesn't match block range"
    );
    assert_eq!(batch.status, expected_status, "SNOS batch status mismatch");
    assert_eq!(batch.starknet_version, expected_version, "SNOS batch starknet_version mismatch");
}

/// Validate SNOS batches align with aggregator batches (for L2)
pub fn assert_snos_aggregator_alignment(snos_batches: &[SnosBatch], aggregator_batches: &[AggregatorBatch]) {
    for snos_batch in snos_batches {
        if let Some(agg_index) = snos_batch.aggregator_batch_index {
            // Find the corresponding aggregator batch
            let agg_batch = aggregator_batches
                .iter()
                .find(|b| b.index == agg_index)
                .unwrap_or_else(|| panic!("Aggregator batch {} not found for SNOS batch {}", agg_index, snos_batch.index));

            // Verify SNOS batch is within aggregator batch range
            assert!(
                snos_batch.start_block >= agg_batch.start_block,
                "SNOS batch {} starts at {} which is before aggregator batch {} start ({})",
                snos_batch.index,
                snos_batch.start_block,
                agg_batch.index,
                agg_batch.start_block
            );

            assert!(
                snos_batch.end_block <= agg_batch.end_block,
                "SNOS batch {} ends at {} which is after aggregator batch {} end ({})",
                snos_batch.index,
                snos_batch.end_block,
                agg_batch.index,
                agg_batch.end_block
            );

            // Verify versions match
            assert_eq!(
                snos_batch.starknet_version, agg_batch.starknet_version,
                "SNOS batch {} version ({:?}) doesn't match aggregator batch {} version ({:?})",
                snos_batch.index, snos_batch.starknet_version, agg_batch.index, agg_batch.starknet_version
            );
        }
    }
}

/// Validate that SNOS batches don't have gaps (for L3 or within aggregator batches)
#[allow(dead_code)]
pub fn assert_snos_no_gaps(batches: &[SnosBatch]) {
    for i in 0..batches.len().saturating_sub(1) {
        let current = &batches[i];
        let next = &batches[i + 1];
        assert_eq!(
            current.end_block + 1,
            next.start_block,
            "Gap detected between SNOS batch {} (ending at {}) and batch {} (starting at {})",
            current.index,
            current.end_block,
            next.index,
            next.start_block
        );
    }
}

/// Validate that SNOS batches don't overlap
#[allow(dead_code)]
pub fn assert_snos_no_overlaps(batches: &[SnosBatch]) {
    for i in 0..batches.len().saturating_sub(1) {
        let current = &batches[i];
        let next = &batches[i + 1];
        assert!(
            current.end_block < next.start_block,
            "Overlap detected: SNOS batch {} ends at {} but batch {} starts at {}",
            current.index,
            current.end_block,
            next.index,
            next.start_block
        );
    }
}
