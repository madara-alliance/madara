use crate::compression::blob::{convert_felt_vec_to_blob_data, state_update_to_blob_data};
use crate::compression::squash::squash;
use crate::core::config::{Config, ConfigParam, StarknetVersion};
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, AggregatorBatchWeights};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::triggers::batching::aggregator::AggregatorState::{Empty, NonEmpty};
use crate::worker::event_handler::triggers::batching::utils::{get_block_builtin_weights, get_block_version};
use crate::worker::event_handler::triggers::batching::BlockProcessingResult;
use crate::worker::utils::biguint_vec_to_u8_vec;
use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use opentelemetry::KeyValue;
use orchestrator_prover_client_interface::Task;
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::{JsonRpcClient, Provider};
use starknet_core::types::MaybePreConfirmedStateUpdate::{PreConfirmedUpdate, Update};
use starknet_core::types::{BlockId, StateUpdate};
use starknet_types_core::felt::Felt;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info};

#[allow(clippy::large_enum_variant)]
pub enum AggregatorState {
    Empty(EmptyAggregatorState),
    NonEmpty(NonEmptyAggregatorState),
}

pub struct EmptyAggregatorState {
    index: u64,
}

pub struct NonEmptyAggregatorState {
    batch: AggregatorBatch,
    blob: StateUpdate,
}

pub struct AggregatorStateHandler {
    config: Arc<Config>,
}

impl AggregatorStateHandler {
    pub fn from_config(config: &Arc<Config>) -> Self {
        Self { config: config.clone() }
    }

    pub async fn load_batch_state(&self) -> Result<AggregatorState, JobError> {
        let batch = self.config.database().get_latest_aggregator_batch().await?;
        if let Some(batch) = batch {
            if batch.status.is_closed() {
                return Ok(Empty(EmptyAggregatorState { index: batch.index + 1 }));
            }
            let state_update_bytes = self.config.storage().get_data(&batch.squashed_state_updates_path).await?;
            let blob: StateUpdate = serde_json::from_slice(&state_update_bytes)?;
            Ok(NonEmpty(NonEmptyAggregatorState::new(batch, blob)))
        } else {
            Ok(Empty(EmptyAggregatorState::new(1)))
        }
    }

    /// Save the given state in DB and Storage
    ///
    /// 1. Update or Create doc in DB
    /// 2. Update or Create the blob and other assets in Storage
    ///
    /// IMPORTANT:
    /// 1. Assuming all the details are already updated in state
    /// 2. Not making database and storage updates atomically. It might happen that one fail and other pass
    pub async fn save_batch_state(&self, state: &NonEmptyAggregatorState) -> Result<(), JobError> {
        info!(batch=?state.batch, "Saving aggregator batch state");
        // Compressing the state update into vector of felts
        // Doing this first since this is dependent on external RPC => Higher chances of failure
        // i.e. if this fails, we won't update anything in our state and prevent data inconsistency
        let compressed_state_update = compress_state_update(
            self.config.madara_rpc_client(),
            &state.blob,
            state.batch.end_block,
            state.batch.starknet_version,
        )
        .await?;

        // Update batch status in the database
        self.config
            .database()
            .update_or_create_aggregator_batch(&state.batch, &AggregatorBatchUpdates::default())
            .await?;

        // Update state update and blob in storage
        self.config
            .storage()
            .put_data(Bytes::from(serde_json::to_string(&state.blob)?), &state.batch.squashed_state_updates_path)
            .await?;
        let blobs = convert_felt_vec_to_blob_data(&compressed_state_update)?;
        for (i, blob) in blobs.iter().enumerate() {
            self.config
                .storage()
                .put_data(
                    biguint_vec_to_u8_vec(blob.as_slice()).into(),
                    &AggregatorBatch::get_blob_file_path(state.batch.index, i as u64 + 1),
                )
                .await?;
        }

        Ok(())
    }
}

pub struct AggregatorBatchLimits {
    pub max_blob_size: usize,
    pub max_batch_size: u64,
    pub max_batch_builtin_weights: AggregatorBatchWeights,
    pub max_batch_time_seconds: u64,
}

impl AggregatorBatchLimits {
    pub fn from_config(config: &ConfigParam) -> Self {
        Self {
            max_blob_size: config.batching_config.max_blob_size,
            max_batch_size: config.batching_config.max_batch_size,
            max_batch_builtin_weights: config.aggregator_batch_weights_limit.clone(),
            max_batch_time_seconds: config.batching_config.max_batch_time_seconds,
        }
    }

    /// Create limits for testing purposes
    #[cfg(test)]
    pub fn new_for_test(
        max_blob_size: usize,
        max_batch_size: u64,
        max_batch_builtin_weights: AggregatorBatchWeights,
        max_batch_time_seconds: u64,
    ) -> Self {
        Self { max_blob_size, max_batch_size, max_batch_builtin_weights, max_batch_time_seconds }
    }
}

pub struct AggregatorHandler {
    config: Arc<Config>,
    limits: AggregatorBatchLimits,
    empty_block_proving_gas: u64,
}

impl AggregatorHandler {
    pub async fn include_block(
        &self,
        block_num: u64,
        state: AggregatorState,
    ) -> Result<BlockProcessingResult<AggregatorState, NonEmptyAggregatorState>, JobError> {
        info!("Including block {} in aggregator batch", block_num);
        // Fetch Starknet version for the current block
        let current_block_starknet_version = get_block_version(block_num, self.config.madara_rpc_client()).await?;

        match state {
            Empty(ref empty_state) => {
                // Get state update for the current block
                let current_state_update = self
                    .config
                    .madara_rpc_client()
                    .get_state_update(BlockId::Number(block_num))
                    .await
                    .map_err(|e| JobError::ProviderError(e.to_string()))?;

                match current_state_update {
                    Update(state_update) => {
                        let compressed_state_update = compress_state_update(
                            self.config.madara_rpc_client(),
                            &state_update,
                            block_num.saturating_sub(1),
                            current_block_starknet_version,
                        )
                        .await?;
                        let new_state = NonEmptyAggregatorState::new(
                            self.start_aggregator_batch(empty_state.index, block_num, compressed_state_update.len())
                                .await?,
                            state_update,
                        );
                        Ok(BlockProcessingResult::Accumulated(new_state))
                    }
                    PreConfirmedUpdate(_) => {
                        info!("Skipping batching for block {} as it is still pending", block_num);
                        Ok(BlockProcessingResult::NotBatched(state))
                    }
                }
            }
            NonEmpty(state) => self.process_block(block_num, state).await,
        }
    }

    async fn process_block(
        &self,
        block_num: u64,
        state: NonEmptyAggregatorState,
    ) -> Result<BlockProcessingResult<AggregatorState, NonEmptyAggregatorState>, JobError> {
        // Fetch block weights for the current block
        let block_weights = AggregatorBatchWeights::from(
            &get_block_builtin_weights(
                block_num,
                self.config.madara_feeder_gateway_client(),
                self.empty_block_proving_gas,
            )
            .await?,
        );

        // Fetch Starknet version of the current block
        let block_version = get_block_version(block_num, self.config.madara_rpc_client()).await?;

        // Get the state update for the block
        let block_state_update = self
            .config
            .madara_rpc_client()
            .get_state_update(BlockId::Number(block_num))
            .await
            .map_err(|e| JobError::ProviderError(e.to_string()))?;

        match block_state_update {
            Update(state_update) => {
                // Squash the state updates

                match state
                    .checked_add_block_with_limits(
                        block_num,
                        &state_update,
                        &block_weights,
                        block_version,
                        &self.limits,
                        self.config.madara_rpc_client(),
                    )
                    .await?
                {
                    Some(updated_state) => {
                        // Can add the given block in this batch
                        Ok(BlockProcessingResult::Accumulated(updated_state))
                    }
                    None => {
                        // Can't add the given block in this batch
                        let completed_state = state.close();

                        let blob_len = compress_state_update(
                            self.config.madara_rpc_client(),
                            &state_update,
                            block_num.saturating_sub(1),
                            block_version,
                        )
                        .await?
                        .len();
                        let new_state = NonEmpty(NonEmptyAggregatorState::new(
                            self.start_aggregator_batch(state.batch.index + 1, block_num, blob_len).await?,
                            state_update,
                        ));
                        Ok(BlockProcessingResult::BatchCompleted { completed_state, new_state })
                    }
                }
            }
            PreConfirmedUpdate(_) => {
                info!("Skipping batching for block {} as it is still pending", block_num);
                Ok(BlockProcessingResult::NotBatched(NonEmpty(state)))
            }
        }
    }

    async fn start_aggregator_batch(
        &self,
        index: u64,
        start_block: u64,
        blob_len: usize,
    ) -> Result<AggregatorBatch, JobError> {
        // Start timing batch creation
        let start_time = Instant::now();

        // Fetch Starknet version for the start block
        // In tests, use a default version if fetch fails due to HTTP mocking limitations
        let starknet_version = get_block_version(start_block, self.config.madara_rpc_client()).await?;
        debug!(
            index = %index,
            start_block = %start_block,
            starknet_version = %starknet_version,
            "Fetched Starknet version for new batch"
        );

        // Start a new bucket
        let bucket_id = self.config.prover_client().submit_task(Task::CreateBucket).await.map_err(|e| {
            error!(bucket_index = %index, error = %e, "Failed to submit create bucket task to prover client, {}", e);
            JobError::Other(OtherError(eyre!(
                "Prover Client Error: Failed to submit create bucket task to prover client, {}",
                e
            )))
        })?;
        debug!(index = %index, bucket_id = %bucket_id, "Created new bucket successfully");

        // Getting the builtin weights for the start_block and adding it in the DB
        let weights = AggregatorBatchWeights::from(
            &get_block_builtin_weights(
                start_block,
                self.config.madara_feeder_gateway_client(),
                self.empty_block_proving_gas,
            )
            .await?,
        );

        let batch = AggregatorBatch::new(index, start_block, bucket_id.clone(), blob_len, weights, starknet_version);

        // Record batch creation time with starknet_version in metrics
        let duration = start_time.elapsed();
        let attributes = [
            KeyValue::new("batch_index", index.to_string()),
            KeyValue::new("start_block", start_block.to_string()),
            KeyValue::new("bucket_id", bucket_id.to_string()),
            KeyValue::new("starknet_version", starknet_version.to_string()),
        ];
        ORCHESTRATOR_METRICS.batch_creation_time.record(duration.as_secs_f64(), &attributes);

        // Update batching rate (batches per hour)
        // This is a simple counter that will be used to calculate rate in Grafana
        ORCHESTRATOR_METRICS.batching_rate.record(1.0, &attributes);

        debug!(
            index = %index,
            duration_seconds = %duration.as_secs_f64(),
            "Batch created successfully"
        );

        Ok(batch)
    }
}

impl EmptyAggregatorState {
    pub fn new(index: u64) -> Self {
        Self { index }
    }
}

/// Result of checking if a block can be added to an aggregator batch
#[derive(Debug, PartialEq)]
pub enum BatchCheckResult {
    /// Block can be added, here are the combined weights
    CanAdd(AggregatorBatchWeights),
    /// Batch is already closed
    BatchClosed,
    /// Starknet version mismatch
    VersionMismatch,
    /// Max blocks per batch reached
    MaxBlocksReached,
    /// Batch time limit exceeded
    TimeLimitExceeded,
    /// Weight overflow during addition
    WeightOverflow,
    /// Combined weights exceed the limit
    WeightLimitExceeded,
}

impl NonEmptyAggregatorState {
    pub fn new(batch: AggregatorBatch, blob: StateUpdate) -> Self {
        Self { batch, blob }
    }

    /// Check if a block can be added to this batch based on synchronous conditions.
    /// This does NOT check blob size (which requires async compression).
    ///
    /// Returns `BatchCheckResult::CanAdd(combined_weights)` if all sync checks pass,
    /// or a specific reason why the block cannot be added.
    pub fn check_block_sync(
        &self,
        block_weights: &AggregatorBatchWeights,
        block_version: StarknetVersion,
        batch_limits: &AggregatorBatchLimits,
    ) -> BatchCheckResult {
        // Check if batch is already closed
        if self.batch.status.is_closed() {
            return BatchCheckResult::BatchClosed;
        }

        // Check version mismatch
        if block_version != self.batch.starknet_version {
            return BatchCheckResult::VersionMismatch;
        }

        // Check max blocks reached
        if self.batch.num_blocks >= batch_limits.max_batch_size {
            return BatchCheckResult::MaxBlocksReached;
        }

        // Check time limit
        let elapsed_seconds = (Utc::now().round_subsecs(0) - self.batch.created_at).abs().num_seconds() as u64;
        if elapsed_seconds >= batch_limits.max_batch_time_seconds {
            return BatchCheckResult::TimeLimitExceeded;
        }

        // Check weight overflow and limit
        match self.batch.builtin_weights.checked_add(block_weights) {
            Some(combined_weights) => {
                if batch_limits.max_batch_builtin_weights.checked_sub(&combined_weights).is_none() {
                    BatchCheckResult::WeightLimitExceeded
                } else {
                    BatchCheckResult::CanAdd(combined_weights)
                }
            }
            None => BatchCheckResult::WeightOverflow,
        }
    }

    pub async fn checked_add_block_with_limits(
        &self,
        block_num: u64,
        block_state_update: &StateUpdate,
        block_weights: &AggregatorBatchWeights,
        block_version: StarknetVersion,
        batch_limits: &AggregatorBatchLimits,
        provider: &Arc<JsonRpcClient<HttpTransport>>,
    ) -> Result<Option<Self>, JobError> {
        // Perform synchronous checks first
        let combined_weights = match self.check_block_sync(block_weights, block_version, batch_limits) {
            BatchCheckResult::CanAdd(weights) => weights,
            _ => return Ok(None),
        };

        // Check compressed state update is within limits (async)
        // Squash state updates
        let squashed_state_update = squash(
            vec![&self.blob, block_state_update],
            if self.batch.start_block == 0 { None } else { Some(self.batch.start_block - 1) },
            provider,
        )
        .await?;
        // Compress the squashed state update
        let compressed_state_update = compress_state_update(
            provider,
            &squashed_state_update,
            block_num.saturating_sub(1),
            self.batch.starknet_version,
        )
        .await?;
        let blob_len = compressed_state_update.len();
        if blob_len > batch_limits.max_blob_size {
            return Ok(None);
        }

        Ok(Some(NonEmptyAggregatorState {
            batch: self.batch.update(block_num, blob_len, combined_weights, None),
            blob: squashed_state_update,
        }))
    }

    pub fn close(&self) -> NonEmptyAggregatorState {
        let mut batch = self.batch.clone();
        batch.status = AggregatorBatchStatus::Closed;
        NonEmptyAggregatorState { batch, blob: self.blob.clone() }
    }
}

impl AggregatorHandler {
    pub fn new(config: Arc<Config>, limits: AggregatorBatchLimits, empty_block_proving_gas: u64) -> AggregatorHandler {
        AggregatorHandler { config, limits, empty_block_proving_gas }
    }
}

// ------ Helper method to compress state update ------

/// Compress the state update and return the blob data (as vector of felts)
async fn compress_state_update(
    provider: &Arc<JsonRpcClient<HttpTransport>>,
    blob: &StateUpdate,
    end_block: u64,
    madara_version: StarknetVersion,
) -> Result<Vec<Felt>, JobError> {
    // Perform stateful compression if needed
    let state_update = if madara_version >= StarknetVersion::V0_13_4 {
        crate::compression::stateful::compress(blob, end_block, provider)
            .await
            .map_err(|err| JobError::Other(OtherError(err)))?
    } else {
        blob.clone()
    };

    // Get a vector of felts from the compressed state update
    let vec_felts = state_update_to_blob_data(state_update, madara_version).await?;

    // Perform stateless compression if needed
    if madara_version >= StarknetVersion::V0_13_3 {
        crate::compression::stateless::compress(&vec_felts).map_err(|err| JobError::Other(OtherError(err)))
    } else {
        Ok(vec_felts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use starknet_core::types::StateDiff;

    /// Helper to create a test batch with configurable parameters
    fn create_test_batch(
        num_blocks: u64,
        status: AggregatorBatchStatus,
        version: StarknetVersion,
        weights: AggregatorBatchWeights,
        created_at: DateTime<Utc>,
    ) -> AggregatorBatch {
        AggregatorBatch {
            id: uuid::Uuid::new_v4(),
            index: 1,
            bucket_id: "test_bucket".to_string(),
            squashed_state_updates_path: "test/path.json".to_string(),
            blob_path: "test/blob".to_string(),
            starknet_version: version,
            start_block: 0,
            end_block: num_blocks.saturating_sub(1),
            num_blocks,
            blob_len: 100,
            builtin_weights: weights,
            status,
            created_at,
            updated_at: Utc::now(),
        }
    }

    /// Helper to create a test state update
    fn create_test_state_update() -> StateUpdate {
        StateUpdate {
            block_hash: Felt::ZERO,
            old_root: Felt::ZERO,
            new_root: Felt::ONE,
            state_diff: StateDiff {
                storage_diffs: vec![],
                deprecated_declared_classes: vec![],
                declared_classes: vec![],
                deployed_contracts: vec![],
                replaced_classes: vec![],
                nonces: vec![],
            },
        }
    }

    /// Helper to create default test limits
    fn create_test_limits() -> AggregatorBatchLimits {
        AggregatorBatchLimits::new_for_test(
            10000,                                         // max_blob_size
            10,                                            // max_batch_size (10 blocks)
            AggregatorBatchWeights::new(1_000_000, 10000), // max weights
            3600,                                          // max_batch_time_seconds (1 hour)
        )
    }

    mod check_block_sync_tests {
        use super::*;

        #[test]
        fn test_batch_closed_returns_batch_closed() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Closed,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::BatchClosed);
        }

        #[test]
        fn test_version_mismatch_returns_version_mismatch() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            // Block has different version than batch
            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_3, &limits);

            assert_eq!(result, BatchCheckResult::VersionMismatch);
        }

        #[test]
        fn test_max_blocks_reached_returns_max_blocks_reached() {
            let batch = create_test_batch(
                10, // Already at max (limit is 10)
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::MaxBlocksReached);
        }

        #[test]
        fn test_time_limit_exceeded_returns_time_limit_exceeded() {
            let old_time = Utc::now() - Duration::seconds(3700); // More than 1 hour ago
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                old_time,
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::TimeLimitExceeded);
        }

        #[test]
        fn test_weight_overflow_returns_weight_overflow() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(usize::MAX, 100), // Near overflow
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(1, 0); // Will overflow

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::WeightOverflow);
        }

        #[test]
        fn test_weight_limit_exceeded_returns_weight_limit_exceeded() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(900_000, 9000), // Near limit
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits(); // limit is 1_000_000, 10000
            let block_weights = AggregatorBatchWeights::new(200_000, 2000); // Would exceed limit

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::WeightLimitExceeded);
        }

        #[test]
        fn test_all_conditions_pass_returns_can_add() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100_000, 1000),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(50_000, 500);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            // Should return CanAdd with combined weights
            match result {
                BatchCheckResult::CanAdd(combined) => {
                    assert_eq!(combined.l1_gas, 150_000);
                    assert_eq!(combined.message_segment_length, 1500);
                }
                other => panic!("Expected CanAdd, got {:?}", other),
            }
        }

        #[test]
        fn test_exactly_at_max_blocks_minus_one_can_add() {
            let batch = create_test_batch(
                9, // One less than max
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert!(matches!(result, BatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_exactly_at_weight_limit_can_add() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(500_000, 5000),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            // Combined will be exactly at limit (1_000_000, 10000)
            let block_weights = AggregatorBatchWeights::new(500_000, 5000);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert!(matches!(result, BatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_just_over_weight_limit_returns_weight_limit_exceeded() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(500_000, 5000),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            // Combined will be just over limit
            let block_weights = AggregatorBatchWeights::new(500_001, 5000);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::WeightLimitExceeded);
        }

        #[test]
        fn test_pending_aggregator_run_status_treated_as_closed() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::PendingAggregatorRun,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::BatchClosed);
        }

        #[test]
        fn test_multiple_version_transitions() {
            // Test all version combinations
            let versions = [StarknetVersion::V0_13_2, StarknetVersion::V0_13_3, StarknetVersion::V0_13_4];

            for batch_version in &versions {
                for block_version in &versions {
                    let batch = create_test_batch(
                        5,
                        AggregatorBatchStatus::Open,
                        *batch_version,
                        AggregatorBatchWeights::new(100, 100),
                        Utc::now(),
                    );
                    let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
                    let limits = create_test_limits();
                    let block_weights = AggregatorBatchWeights::new(100, 100);

                    let result = state.check_block_sync(&block_weights, *block_version, &limits);

                    if batch_version == block_version {
                        assert!(
                            matches!(result, BatchCheckResult::CanAdd(_)),
                            "Same version {:?} should allow add",
                            batch_version
                        );
                    } else {
                        assert_eq!(
                            result,
                            BatchCheckResult::VersionMismatch,
                            "Different versions {:?} vs {:?} should mismatch",
                            batch_version,
                            block_version
                        );
                    }
                }
            }
        }
    }

    mod close_tests {
        use super::*;

        #[test]
        fn test_close_changes_status_to_closed() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());

            let closed_state = state.close();

            assert_eq!(closed_state.batch.status, AggregatorBatchStatus::Closed);
        }

        #[test]
        fn test_close_preserves_other_fields() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let original_index = batch.index;
            let original_start_block = batch.start_block;
            let original_end_block = batch.end_block;
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());

            let closed_state = state.close();

            assert_eq!(closed_state.batch.index, original_index);
            assert_eq!(closed_state.batch.start_block, original_start_block);
            assert_eq!(closed_state.batch.end_block, original_end_block);
        }
    }

    mod empty_state_tests {
        use super::*;

        #[test]
        fn test_empty_state_new() {
            let state = EmptyAggregatorState::new(5);
            assert_eq!(state.index, 5);
        }
    }

    mod limits_tests {
        use super::*;

        #[test]
        fn test_limits_new_for_test() {
            let limits = AggregatorBatchLimits::new_for_test(5000, 20, AggregatorBatchWeights::new(100, 200), 7200);

            assert_eq!(limits.max_blob_size, 5000);
            assert_eq!(limits.max_batch_size, 20);
            assert_eq!(limits.max_batch_builtin_weights.l1_gas, 100);
            assert_eq!(limits.max_batch_builtin_weights.message_segment_length, 200);
            assert_eq!(limits.max_batch_time_seconds, 7200);
        }
    }
}
