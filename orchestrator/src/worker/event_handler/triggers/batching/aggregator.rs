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
    max_blob_size: usize,
    max_batch_size: u64,
    max_batch_builtin_weights: AggregatorBatchWeights,
    max_batch_time_seconds: u64,
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

impl NonEmptyAggregatorState {
    pub fn new(batch: AggregatorBatch, blob: StateUpdate) -> Self {
        Self { batch, blob }
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
        // Check batch status, size and time
        if self.batch.status.is_closed()
            || block_version != self.batch.starknet_version
            || self.batch.num_blocks >= batch_limits.max_batch_size
            || (Utc::now().round_subsecs(0) - self.batch.created_at).abs().num_seconds() as u64
                >= batch_limits.max_batch_time_seconds
        {
            return Ok(None);
        }

        // Check combined weights don't exceed max cap and limit
        let combined_weights = match self.batch.builtin_weights.checked_add(block_weights) {
            Some(combined_weights) => {
                if batch_limits.max_batch_builtin_weights.checked_sub(&combined_weights).is_none() {
                    return Ok(None);
                } else {
                    combined_weights
                }
            }
            None => {
                return Ok(None);
            }
        };

        // Check compressed state update is within limits
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
