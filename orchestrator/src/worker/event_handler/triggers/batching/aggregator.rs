use crate::compression::batch_rpc::BatchRpcClient;
use crate::compression::blob::{convert_felt_vec_to_blob_data, state_update_to_blob_data};
use crate::compression::squash::squash;
use crate::core::config::{Config, ConfigParam, ProverKind, StarknetVersion, SUPPORTED_STARKNET_VERSION};
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{AggregatorBatch, AggregatorBatchStatus, AggregatorBatchUpdates, AggregatorBatchWeights};
use crate::types::constant::ORCHESTRATOR_VERSION;
use crate::types::jobs::types::JobType;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::utils::provider_retry::retry_provider_read;
use crate::worker::event_handler::triggers::batching::aggregator::AggregatorState::{Empty, NonEmpty};
use crate::worker::event_handler::triggers::batching::utils::{get_block_builtin_weights, get_block_version};
use crate::worker::event_handler::triggers::batching::BlockProcessingResult;
use crate::worker::utils::biguint_vec_to_u8_vec;
use bytes::Bytes;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use opentelemetry::KeyValue;
use orchestrator_prover_client_interface::Task;
use starknet::providers::Provider;
use starknet_core::types::MaybePreConfirmedStateUpdate::{PreConfirmedUpdate, Update};
use starknet_core::types::{BlockId, MaybePreConfirmedStateUpdate, StateUpdate};
use starknet_types_core::felt::Felt;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, error, info, warn};

const AGGREGATOR_N_TASKS_WORDS: usize = 1;
const AGGREGATOR_CHILD_WRAPPER_WORDS: usize = 2;
const SNOS_OUTPUT_FIXED_WORDS: usize = 14;
const SNOS_FULL_CONTRACT_HEADER_WORDS: usize = 6;
const SNOS_FULL_STORAGE_UPDATE_WORDS: usize = 3;
const SNOS_FULL_CLASS_UPDATE_WORDS: usize = 3;

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
    /// 2. Not making database and storage updates atomically. It might happen that one fails and the other passes
    pub async fn save_batch_state(&self, state: &NonEmptyAggregatorState) -> Result<(), JobError> {
        let save_started_at = Instant::now();
        info!(batch=?state.batch, "Saving aggregator batch state");
        // Compressing the state update into vector of felts
        // Doing this first since this is dependent on external RPC => Higher chances of failure
        // i.e. if this fails, we won't update anything in our state and prevent data inconsistency
        let compress_started_at = Instant::now();
        let compressed_state_update = compress_state_update(
            &state.blob,
            state.batch.end_block,
            state.batch.starknet_version,
            self.config.batch_rpc_client(),
        )
        .await?;
        info!(
            batch_index = %state.batch.index,
            end_block = %state.batch.end_block,
            blob_len = compressed_state_update.len(),
            duration_ms = %compress_started_at.elapsed().as_millis(),
            "Compressed aggregator batch state for save"
        );

        // Update batch status in the database
        self.config
            .database()
            .update_or_create_aggregator_batch(&state.batch, &AggregatorBatchUpdates::default())
            .await?;
        MetricsRecorder::record_aggregator_input_size_upper_bound(
            &state.batch.status.to_string(),
            state.batch.aggregator_input_size_upper_bound,
        );

        // Update state update and blob in storage
        self.config
            .storage()
            .put_data(Bytes::from(serde_json::to_string(&state.blob)?), &state.batch.squashed_state_updates_path)
            .await?;
        let blobs = convert_felt_vec_to_blob_data(&compressed_state_update)?;
        for (i, blob) in blobs.iter().enumerate() {
            let path = AggregatorBatch::get_blob_file_path(state.batch.index, i as u64 + 1);
            self.config.storage().put_data(biguint_vec_to_u8_vec(blob.as_slice()).into(), &path).await?;
        }
        info!(
            batch_index = %state.batch.index,
            blob_count = blobs.len(),
            duration_ms = %save_started_at.elapsed().as_millis(),
            "Saved aggregator batch state"
        );

        Ok(())
    }
}

pub struct AggregatorBatchConfig {
    pub max_blob_size: usize,
    pub max_batch_size: u64,
    pub max_batch_builtin_weights: AggregatorBatchWeights,
    pub max_batch_time_seconds: u64,
    pub empty_block_proving_gas: u64,
    pub max_aggregator_input_size: usize,
}

impl AggregatorBatchConfig {
    pub fn from_config(config: &ConfigParam) -> Self {
        Self {
            max_blob_size: config.batching_config.max_blob_size,
            max_batch_size: config.batching_config.max_batch_size,
            max_batch_builtin_weights: config.aggregator_batch_weights_limit.clone(),
            max_batch_time_seconds: config.batching_config.max_batch_time_seconds,
            empty_block_proving_gas: config.batching_config.default_empty_block_proving_gas,
            max_aggregator_input_size: config.batching_config.max_aggregator_input_size,
        }
    }

    /// Create limits for testing purposes
    #[cfg(test)]
    pub fn new_for_test(
        max_blob_size: usize,
        max_batch_size: u64,
        max_batch_builtin_weights: AggregatorBatchWeights,
        max_batch_time_seconds: u64,
        empty_block_proving_gas: u64,
        max_aggregator_input_size: usize,
    ) -> Self {
        Self {
            max_blob_size,
            max_batch_size,
            max_batch_builtin_weights,
            max_batch_time_seconds,
            empty_block_proving_gas,
            max_aggregator_input_size,
        }
    }
}

pub struct AggregatorHandler {
    config: Arc<Config>,
    batch_config: AggregatorBatchConfig,
}

impl AggregatorHandler {
    pub async fn include_block(
        &self,
        block_num: u64,
        state: AggregatorState,
    ) -> Result<BlockProcessingResult<AggregatorState, NonEmptyAggregatorState>, JobError> {
        let state_kind = match &state {
            Empty(_) => "empty",
            NonEmpty(_) => "non_empty",
        };
        info!(block_num, state = state_kind, "Including block in aggregator batch");
        // Fetch Starknet version for the current block
        let current_block_starknet_version = self.fetch_block_version(block_num, "include_block").await?;

        // Check if block's Starknet version is supported (applies to all states)
        if !current_block_starknet_version.is_supported() {
            tracing::warn!(
                block_num = %block_num,
                version = %current_block_starknet_version,
                supported = %SUPPORTED_STARKNET_VERSION,
                "Block has unsupported Starknet version. Closing current batch and halting aggregator batching. \
                 Manual intervention required — update orchestrator to support version {} and redeploy.",
                current_block_starknet_version,
            );
            // Close the current batch if it has blocks, so they get processed
            if let NonEmpty(mut non_empty) = state {
                non_empty.close();
                return Ok(BlockProcessingResult::NotBatched(NonEmpty(non_empty)));
            }
            return Ok(BlockProcessingResult::NotBatched(state));
        }

        match state {
            Empty(empty_state) => {
                // Get state update for the current block
                let current_state_update = self.fetch_state_update(block_num, "empty_state").await?;

                match current_state_update {
                    Update(state_update) => {
                        let block_weights = self.fetch_block_weights(block_num, "empty_state").await?;
                        let aggregator_input_size_upper_bound = initial_aggregator_input_size_upper_bound(
                            &state_update,
                            block_weights.message_segment_length,
                        )?;
                        if aggregator_input_size_upper_bound > self.batch_config.max_aggregator_input_size {
                            warn!(
                                block_num = %block_num,
                                aggregator_input_size_upper_bound = %aggregator_input_size_upper_bound,
                                max_aggregator_input_size = %self.batch_config.max_aggregator_input_size,
                                "Stopping before oversized block while aggregator batch state is empty"
                            );
                            return Ok(BlockProcessingResult::NotBatched(Empty(empty_state)));
                        }

                        let compressed_state_update = compress_state_update(
                            &state_update,
                            block_num.saturating_sub(1),
                            current_block_starknet_version,
                            self.config.batch_rpc_client(),
                        )
                        .await?;
                        let new_state = NonEmptyAggregatorState::new(
                            self.start_aggregator_batch(
                                empty_state.index,
                                block_num,
                                compressed_state_update.len(),
                                &state_update,
                            )
                            .await?,
                            state_update,
                        );
                        Ok(BlockProcessingResult::Accumulated(new_state))
                    }
                    PreConfirmedUpdate(_) => {
                        info!("Skipping batching for block {} as it is still pending", block_num);
                        Ok(BlockProcessingResult::NotBatched(Empty(empty_state)))
                    }
                }
            }
            NonEmpty(state) => self.process_block(block_num, state).await,
        }
    }

    async fn fetch_block_version(&self, block_num: u64, operation: &str) -> Result<StarknetVersion, JobError> {
        let started_at = Instant::now();
        let version = get_block_version(block_num, self.config.madara_rpc_client()).await?;
        info!(
            block_num,
            operation,
            version = %version,
            duration_ms = %started_at.elapsed().as_millis(),
            "Fetched aggregator block Starknet version"
        );
        Ok(version)
    }

    async fn fetch_block_weights(&self, block_num: u64, operation: &str) -> Result<AggregatorBatchWeights, JobError> {
        let started_at = Instant::now();
        let weights = AggregatorBatchWeights::from(
            &get_block_builtin_weights(
                block_num,
                self.config.madara_feeder_gateway_client(),
                self.batch_config.empty_block_proving_gas,
            )
            .await?,
        );
        info!(
            block_num,
            operation,
            l1_gas = %weights.l1_gas,
            message_segment_length = %weights.message_segment_length,
            duration_ms = %started_at.elapsed().as_millis(),
            "Fetched aggregator block bouncer weights"
        );
        Ok(weights)
    }

    async fn fetch_state_update(
        &self,
        block_num: u64,
        operation: &str,
    ) -> Result<MaybePreConfirmedStateUpdate, JobError> {
        let started_at = Instant::now();
        let provider = self.config.madara_rpc_client();
        let update =
            retry_provider_read("madara_get_state_update", || provider.get_state_update(BlockId::Number(block_num)))
                .await
                .map_err(|e| JobError::ProviderError(e.to_string()))?;
        match &update {
            Update(state_update) => {
                let (modified_contracts, storage_updates, declared_classes) =
                    state_update_full_output_counts(state_update);
                info!(
                    block_num,
                    operation,
                    modified_contracts,
                    storage_updates,
                    declared_classes,
                    duration_ms = %started_at.elapsed().as_millis(),
                    "Fetched aggregator state update"
                );
            }
            PreConfirmedUpdate(_) => {
                info!(
                    block_num,
                    operation,
                    duration_ms = %started_at.elapsed().as_millis(),
                    "Fetched pre-confirmed aggregator state update"
                );
            }
        }
        Ok(update)
    }

    async fn process_block(
        &self,
        block_num: u64,
        mut state: NonEmptyAggregatorState,
    ) -> Result<BlockProcessingResult<AggregatorState, NonEmptyAggregatorState>, JobError> {
        // Gap detection - check if block_num is exactly end_block + 1
        if block_num != state.batch.end_block + 1 {
            tracing::warn!(
                expected_block = state.batch.end_block + 1,
                actual_block = block_num,
                batch_index = state.batch.index,
                "Gap detected in block sequence, closing batch"
            );
            state.close();
            return Ok(BlockProcessingResult::NotBatched(NonEmpty(state)));
        }

        // Fetch block weights for the current block
        let block_weights = self.fetch_block_weights(block_num, "non_empty_state").await?;

        // Fetch Starknet version of the current block
        let block_version = self.fetch_block_version(block_num, "non_empty_state").await?;

        // Get the state update for the block
        let block_state_update = self.fetch_state_update(block_num, "non_empty_state").await?;

        match block_state_update {
            Update(state_update) => {
                // Squash the state updates

                match state
                    .checked_add_block_with_limits(
                        block_num,
                        &state_update,
                        &block_weights,
                        block_version,
                        &self.batch_config,
                        self.config.batch_rpc_client(),
                    )
                    .await?
                {
                    Some(updated_state) => {
                        // Can add the given block in this batch
                        Ok(BlockProcessingResult::Accumulated(updated_state))
                    }
                    None => {
                        // Can't add the given block in this batch
                        state.close();
                        let single_block_aggregator_input_size_upper_bound = initial_aggregator_input_size_upper_bound(
                            &state_update,
                            block_weights.message_segment_length,
                        )?;
                        if single_block_aggregator_input_size_upper_bound > self.batch_config.max_aggregator_input_size
                        {
                            warn!(
                                batch_index = %state.batch.index,
                                block_num = %block_num,
                                aggregator_input_size_upper_bound = %single_block_aggregator_input_size_upper_bound,
                                max_aggregator_input_size = %self.batch_config.max_aggregator_input_size,
                                "Closing current aggregator batch and stopping before oversized next block"
                            );
                            return Ok(BlockProcessingResult::NotBatched(NonEmpty(state)));
                        }

                        let blob_len = compress_state_update(
                            &state_update,
                            block_num.saturating_sub(1),
                            block_version,
                            self.config.batch_rpc_client(),
                        )
                        .await?
                        .len();
                        let new_state = NonEmpty(NonEmptyAggregatorState::new(
                            self.start_aggregator_batch(state.batch.index + 1, block_num, blob_len, &state_update)
                                .await?,
                            state_update,
                        ));
                        Ok(BlockProcessingResult::BatchCompleted { completed_state: state, new_state })
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
        state_update: &StateUpdate,
    ) -> Result<AggregatorBatch, JobError> {
        // Fetch Starknet version for the start block
        // In tests, use a default version if fetch fails due to HTTP mocking limitations
        let starknet_version = self.fetch_block_version(start_block, "start_aggregator_batch").await?;

        // Getting the builtin weights for the start_block and adding it in the DB
        let weights = self.fetch_block_weights(start_block, "start_aggregator_batch").await?;

        let aggregator_input_size_upper_bound =
            initial_aggregator_input_size_upper_bound(state_update, weights.message_segment_length)?;
        if aggregator_input_size_upper_bound > self.batch_config.max_aggregator_input_size {
            return Err(JobError::Other(OtherError(eyre!(
                "Block {} aggregator input size upper bound {} exceeds the maximum allowed size {}",
                start_block,
                aggregator_input_size_upper_bound,
                self.batch_config.max_aggregator_input_size
            ))));
        }

        let bucket_id = match self.config.prover_kind() {
            ProverKind::Atlantic => {
                let bucket_id = self.config.prover_client().submit_task(Task::CreateBucket).await.map_err(|e| {
                    error!(bucket_index = %index, error = %e, "Failed to submit create bucket task to prover client, {}", e);
                    JobError::Other(OtherError(eyre!(
                        "Prover Client Error: Failed to submit create bucket task to prover client, {}",
                        e
                    )))
                })?;
                debug!(index = %index, bucket_id = %bucket_id, "Created new Atlantic bucket successfully");
                Some(bucket_id)
            }
            ProverKind::Sharp | ProverKind::Mock => None,
        };

        let batch = AggregatorBatch::new(
            index,
            start_block,
            bucket_id,
            blob_len,
            aggregator_input_size_upper_bound,
            weights,
            starknet_version,
        );

        // Record batch creation count
        let attributes = [
            KeyValue::new("operation_job_type", format!("{:?}", JobType::Aggregator)),
            KeyValue::new("starknet_version", starknet_version.to_string()),
        ];
        // "Batching rate" is derived in PromQL/Grafana from this counter.
        ORCHESTRATOR_METRICS.batch_creation_total.add(1.0, &attributes);

        debug!(index = %index, "Batch created successfully");

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
    StarknetVersionMismatch,
    /// Block's Starknet version is not supported by this orchestrator
    StarknetVersionUnsupported,
    /// Batch was created by a different orchestrator version
    OrchestratorVersionMismatch,
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

    pub fn batch_index(&self) -> u64 {
        self.batch.index
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
        batch_limits: &AggregatorBatchConfig,
    ) -> BatchCheckResult {
        // Check if batch is already closed
        if self.batch.status.is_closed() {
            return BatchCheckResult::BatchClosed;
        }

        // Check if batch was created by the current orchestrator version
        if self.batch.orchestrator_version != ORCHESTRATOR_VERSION {
            return BatchCheckResult::OrchestratorVersionMismatch;
        }

        // Check version mismatch
        if block_version != self.batch.starknet_version {
            return BatchCheckResult::StarknetVersionMismatch;
        }

        // Check if block version is supported by this orchestrator
        if !block_version.is_supported() {
            return BatchCheckResult::StarknetVersionUnsupported;
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
        batch_limits: &AggregatorBatchConfig,
        batch_client: &BatchRpcClient,
    ) -> Result<Option<Self>, JobError> {
        let block_aggregator_input_size_upper_bound =
            aggregator_child_input_size_upper_bound(block_state_update, block_weights.message_segment_length)?;
        let aggregator_input_size_upper_bound = self
            .batch
            .aggregator_input_size_upper_bound
            .checked_add(block_aggregator_input_size_upper_bound)
            .ok_or_else(|| {
                JobError::Other(OtherError(eyre!(
                    "Aggregator input size upper bound overflow for batch {} while adding block {}",
                    self.batch.index,
                    block_num
                )))
            })?;
        if aggregator_input_size_upper_bound > batch_limits.max_aggregator_input_size {
            debug!(
                batch_index = %self.batch.index,
                block_num = %block_num,
                aggregator_input_size_upper_bound = %aggregator_input_size_upper_bound,
                max_aggregator_input_size = %batch_limits.max_aggregator_input_size,
                "Closing aggregator batch"
            );
            return Ok(None);
        }

        // Perform synchronous checks first
        let check_result = self.check_block_sync(block_weights, block_version, batch_limits);
        let combined_weights = match check_result {
            BatchCheckResult::CanAdd(weights) => weights,
            reason => {
                debug!(
                    batch_index = %self.batch.index,
                    block_num = %block_num,
                    reason = ?reason,
                    "Closing aggregator batch"
                );
                return Ok(None);
            }
        };

        // Check compressed state update is within limits (async)
        // Squash state updates
        let pre_range_block = if self.batch.start_block == 0 { None } else { Some(self.batch.start_block - 1) };
        let squash_started_at = Instant::now();
        let squashed_state_update = squash(vec![&self.blob, block_state_update], pre_range_block, batch_client).await?;
        let (squashed_modified_contracts, squashed_storage_updates, squashed_declared_classes) =
            state_update_full_output_counts(&squashed_state_update);
        info!(
            batch_index = %self.batch.index,
            block_num,
            ?pre_range_block,
            modified_contracts = squashed_modified_contracts,
            storage_updates = squashed_storage_updates,
            declared_classes = squashed_declared_classes,
            duration_ms = %squash_started_at.elapsed().as_millis(),
            "Squashed aggregator state updates"
        );
        // Compress the squashed state update
        let compress_started_at = Instant::now();
        let compressed_state_update = compress_state_update(
            &squashed_state_update,
            block_num.saturating_sub(1),
            self.batch.starknet_version,
            batch_client,
        )
        .await?;
        info!(
            batch_index = %self.batch.index,
            block_num,
            blob_len = compressed_state_update.len(),
            duration_ms = %compress_started_at.elapsed().as_millis(),
            "Compressed squashed aggregator state update"
        );
        let blob_len = compressed_state_update.len();
        if blob_len > batch_limits.max_blob_size {
            debug!(
                batch_index = %self.batch.index,
                block_num = %block_num,
                blob_len = %blob_len,
                max_blob_size = %batch_limits.max_blob_size,
                "Closing aggregator batch: BlobSizeExceeded"
            );
            return Ok(None);
        }

        Ok(Some(NonEmptyAggregatorState {
            batch: self.batch.update(block_num, blob_len, aggregator_input_size_upper_bound, combined_weights, None),
            blob: squashed_state_update,
        }))
    }

    pub fn close(&mut self) {
        self.batch.status = AggregatorBatchStatus::Closed;
    }
}

impl AggregatorHandler {
    pub fn new(config: Arc<Config>, batch_config: AggregatorBatchConfig) -> AggregatorHandler {
        AggregatorHandler { config, batch_config }
    }
}

fn initial_aggregator_input_size_upper_bound(
    state_update: &StateUpdate,
    message_segment_length: usize,
) -> Result<usize, JobError> {
    let (modified_contracts, storage_updates, declared_classes) = state_update_full_output_counts(state_update);
    aggregator_input_size_from_counts(1, message_segment_length, modified_contracts, storage_updates, declared_classes)
}

fn aggregator_child_input_size_upper_bound(
    state_update: &StateUpdate,
    message_segment_length: usize,
) -> Result<usize, JobError> {
    let (modified_contracts, storage_updates, declared_classes) = state_update_full_output_counts(state_update);
    aggregator_input_size_from_counts(1, message_segment_length, modified_contracts, storage_updates, declared_classes)?
        .checked_sub(AGGREGATOR_N_TASKS_WORDS)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator child input size upper bound underflow"))))
}

/// Conservative aggregator input size upper bound in felts.
///
/// Formula:
/// `n_tasks + n_children * (child_wrapper_words + snos_output_fixed_words) + message_segment_length
/// + modified_contracts * full_contract_header_words
/// + storage_updates * full_storage_update_words
/// + declared_classes * full_class_update_words`.
///
/// This intentionally counts each child's full output shape instead of trying to model squashing
/// savings, so the guard closes batches before the aggregator bootloader input can exceed the
/// configured limit.
fn aggregator_input_size_from_counts(
    n_children: usize,
    message_segment_length: usize,
    modified_contracts: usize,
    storage_updates: usize,
    declared_classes: usize,
) -> Result<usize, JobError> {
    let per_child_fixed_words = SNOS_OUTPUT_FIXED_WORDS
        .checked_add(AGGREGATOR_CHILD_WRAPPER_WORDS)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator per-child fixed size overflow"))))?;
    let fixed_words = per_child_fixed_words
        .checked_mul(n_children)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator fixed input size overflow"))))?;
    let contract_words = modified_contracts
        .checked_mul(SNOS_FULL_CONTRACT_HEADER_WORDS)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator contract input size overflow"))))?;
    let storage_words = storage_updates
        .checked_mul(SNOS_FULL_STORAGE_UPDATE_WORDS)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator storage input size overflow"))))?;
    let class_words = declared_classes
        .checked_mul(SNOS_FULL_CLASS_UPDATE_WORDS)
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator class input size overflow"))))?;

    AGGREGATOR_N_TASKS_WORDS
        .checked_add(fixed_words)
        .and_then(|size| size.checked_add(message_segment_length))
        .and_then(|size| size.checked_add(contract_words))
        .and_then(|size| size.checked_add(storage_words))
        .and_then(|size| size.checked_add(class_words))
        .ok_or_else(|| JobError::Other(OtherError(eyre!("Aggregator input size overflow"))))
}

fn state_update_full_output_counts(state_update: &StateUpdate) -> (usize, usize, usize) {
    let state_diff = &state_update.state_diff;
    let storage_updates = state_diff.storage_diffs.iter().map(|diff| diff.storage_entries.len()).sum::<usize>();
    let modified_contracts = count_modified_contracts(state_update);
    let declared_classes = state_diff.declared_classes.len()
        + state_diff.deprecated_declared_classes.len()
        + state_diff.migrated_compiled_classes.as_ref().map_or(0, Vec::len);

    (modified_contracts, storage_updates, declared_classes)
}

fn count_modified_contracts(state_update: &StateUpdate) -> usize {
    let state_diff = &state_update.state_diff;
    let mut contracts = HashSet::new();

    contracts.extend(state_diff.storage_diffs.iter().map(|diff| diff.address));
    contracts.extend(state_diff.nonces.iter().map(|nonce| nonce.contract_address));
    contracts.extend(state_diff.deployed_contracts.iter().map(|contract| contract.address));
    contracts.extend(state_diff.replaced_classes.iter().map(|class| class.contract_address));

    contracts.len()
}

// ------ Helper method to compress state update ------

/// Compress the state update and return the blob data (as vector of felts)
async fn compress_state_update(
    blob: &StateUpdate,
    end_block: u64,
    madara_version: StarknetVersion,
    batch_client: &BatchRpcClient,
) -> Result<Vec<Felt>, JobError> {
    let started_at = Instant::now();
    // Perform stateful compression if needed
    let state_update = if madara_version >= StarknetVersion::V0_13_4 {
        let stateful_started_at = Instant::now();
        let state_update = crate::compression::stateful::compress(blob, end_block, batch_client)
            .await
            .map_err(|err| JobError::Other(OtherError(err)))?;
        info!(
            end_block,
            duration_ms = %stateful_started_at.elapsed().as_millis(),
            "Applied stateful compression to aggregator state update"
        );
        state_update
    } else {
        blob.clone()
    };

    // Get a vector of felts from the compressed state update
    let vec_felts = state_update_to_blob_data(state_update, madara_version).await?;

    // Perform stateless compression if needed
    let compressed = if madara_version >= StarknetVersion::V0_13_3 {
        crate::compression::stateless::compress(&vec_felts).map_err(|err| JobError::Other(OtherError(err)))?
    } else {
        vec_felts
    };
    info!(
        end_block,
        felt_count = compressed.len(),
        duration_ms = %started_at.elapsed().as_millis(),
        "Compressed aggregator state update"
    );
    Ok(compressed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration, Utc};
    use starknet_core::types::StateDiff;

    const TEST_MAX_AGGREGATOR_INPUT_SIZE: usize = 1_000;

    /// Helper to create a test batch with configurable parameters
    fn create_test_batch(
        num_blocks: u64,
        status: AggregatorBatchStatus,
        version: StarknetVersion,
        weights: AggregatorBatchWeights,
        created_at: DateTime<Utc>,
    ) -> AggregatorBatch {
        create_test_batch_with_orchestrator_version(
            num_blocks,
            status,
            version,
            weights,
            created_at,
            ORCHESTRATOR_VERSION.to_string(),
        )
    }

    /// Helper to create a test batch with a custom orchestrator version
    fn create_test_batch_with_orchestrator_version(
        num_blocks: u64,
        status: AggregatorBatchStatus,
        version: StarknetVersion,
        weights: AggregatorBatchWeights,
        created_at: DateTime<Utc>,
        orchestrator_version: String,
    ) -> AggregatorBatch {
        AggregatorBatch {
            id: uuid::Uuid::new_v4(),
            index: 1,
            orchestrator_version,
            bucket_id: Some("test_bucket".to_string()),
            squashed_state_updates_path: "test/path.json".to_string(),
            blob_path: "test/blob".to_string(),
            starknet_version: version,
            start_block: 0,
            end_block: num_blocks.saturating_sub(1),
            num_blocks,
            blob_len: 100,
            aggregator_input_size_upper_bound: 1,
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
                migrated_compiled_classes: None,
            },
        }
    }

    /// Helper to create default test limits
    fn create_test_limits() -> AggregatorBatchConfig {
        AggregatorBatchConfig::new_for_test(
            10000,                                         // max_blob_size
            10,                                            // max_batch_size (10 blocks)
            AggregatorBatchWeights::new(1_000_000, 10000), // max weights
            3600,                                          // max_batch_time_seconds (1 hour)
            100,                                           // empty block's proving gas
            TEST_MAX_AGGREGATOR_INPUT_SIZE,
        )
    }

    mod aggregator_input_size_tests {
        use super::*;
        use crate::tests::config::{ConfigType, MockType, TestConfigBuilder};
        use blockifier::bouncer::BouncerWeights;
        use httpmock::MockServer;
        use rstest::rstest;
        use serde_json::json;
        use starknet_api::execution_resources::GasAmount;
        use starknet_core::types::{
            BlockStatus, BlockWithTxHashes, ContractStorageDiffItem, DeclaredClassItem, DeployedContractItem,
            L1DataAvailabilityMode, MigratedCompiledClassItem, NonceUpdate, ReplacedClassItem, ResourcePrice,
            StorageEntry,
        };
        use url::Url;

        #[rstest]
        #[case(110, 0, 451, 287_339, 0, 866_484)]
        #[case(1, 7, 4, 3, 4, 69)]
        fn test_aggregator_input_size_formula(
            #[case] n_children: usize,
            #[case] message_segment_length: usize,
            #[case] modified_contracts: usize,
            #[case] storage_updates: usize,
            #[case] declared_classes: usize,
            #[case] expected: usize,
        ) {
            let input_size = aggregator_input_size_from_counts(
                n_children,
                message_segment_length,
                modified_contracts,
                storage_updates,
                declared_classes,
            )
            .unwrap();

            assert_eq!(input_size, expected);
        }

        #[test]
        fn test_state_update_full_output_size_counts_all_terms() {
            let mut state_update = create_test_state_update();
            state_update.state_diff.storage_diffs = vec![
                ContractStorageDiffItem {
                    address: Felt::from(1),
                    storage_entries: vec![
                        StorageEntry { key: Felt::from(10), value: Felt::from(11) },
                        StorageEntry { key: Felt::from(12), value: Felt::from(13) },
                    ],
                },
                ContractStorageDiffItem {
                    address: Felt::from(2),
                    storage_entries: vec![StorageEntry { key: Felt::from(20), value: Felt::from(21) }],
                },
            ];
            state_update.state_diff.nonces =
                vec![NonceUpdate { contract_address: Felt::from(3), nonce: Felt::from(30) }];
            state_update.state_diff.deployed_contracts =
                vec![DeployedContractItem { address: Felt::from(4), class_hash: Felt::from(40) }];
            state_update.state_diff.replaced_classes =
                vec![ReplacedClassItem { contract_address: Felt::from(2), class_hash: Felt::from(200) }];
            state_update.state_diff.declared_classes = vec![
                DeclaredClassItem { class_hash: Felt::from(50), compiled_class_hash: Felt::from(51) },
                DeclaredClassItem { class_hash: Felt::from(52), compiled_class_hash: Felt::from(53) },
            ];
            state_update.state_diff.deprecated_declared_classes = vec![Felt::from(60)];
            state_update.state_diff.migrated_compiled_classes = Some(vec![MigratedCompiledClassItem {
                class_hash: Felt::from(70),
                compiled_class_hash: Felt::from(71),
            }]);

            assert_eq!(aggregator_child_input_size_upper_bound(&state_update, 7).unwrap(), 68);
            assert_eq!(initial_aggregator_input_size_upper_bound(&state_update, 7).unwrap(), 69);
        }

        #[tokio::test]
        async fn test_include_block_stops_without_bucket_when_empty_batch_exceeds_input_limit() {
            let server = MockServer::start();
            let block_num = 6;
            let provider_url = format!("http://localhost:{}", server.port());

            server.mock(|when, then| {
                when.path("/")
                    .body_includes("starknet_getBlockWithTxHashes")
                    .body_includes(format!(r#""block_number":{}"#, block_num));
                then.status(200).body(
                    serde_json::to_vec(&json!({
                        "jsonrpc": "2.0",
                        "result": test_block_with_version(block_num, "0.14.2"),
                        "id": 1
                    }))
                    .unwrap(),
                );
            });

            let block_state_update = create_test_state_update();
            server.mock(|when, then| {
                when.path("/").body_includes("starknet_getStateUpdate");
                then.status(200).body(
                    serde_json::to_vec(&json!({
                        "jsonrpc": "2.0",
                        "result": block_state_update,
                        "id": 1
                    }))
                    .unwrap(),
                );
            });

            server.mock(|when, then| {
                when.path("/feeder_gateway/get_block_bouncer_weights");
                then.status(200).body(serde_json::to_vec(&oversized_bouncer_weights()).unwrap());
            });

            let services = TestConfigBuilder::new()
                .configure_rpc_url(ConfigType::Mock(MockType::RpcUrl(Url::parse(&provider_url).unwrap())))
                .configure_madara_feeder_gateway_url(&provider_url)
                .build()
                .await;

            let handler = AggregatorHandler::new(services.config, create_test_limits());
            let state = Empty(EmptyAggregatorState::new(1));

            let result = handler.include_block(block_num, state).await.unwrap();

            assert!(matches!(result, BlockProcessingResult::NotBatched(Empty(_))));
        }

        fn oversized_bouncer_weights() -> BouncerWeights {
            BouncerWeights {
                l1_gas: 1,
                message_segment_length: TEST_MAX_AGGREGATOR_INPUT_SIZE,
                n_events: 0,
                n_txs: 0,
                state_diff_size: 0,
                sierra_gas: GasAmount(0),
                proving_gas: GasAmount(1),
                receipt_l2_gas: GasAmount(0),
            }
        }

        fn test_block_with_version(block_num: u64, starknet_version: &str) -> serde_json::Value {
            serde_json::to_value(BlockWithTxHashes {
                status: BlockStatus::AcceptedOnL1,
                block_hash: Default::default(),
                parent_hash: Default::default(),
                block_number: block_num,
                new_root: Default::default(),
                timestamp: 0,
                sequencer_address: Default::default(),
                l1_gas_price: ResourcePrice { price_in_fri: Default::default(), price_in_wei: Default::default() },
                l2_gas_price: ResourcePrice { price_in_fri: Default::default(), price_in_wei: Default::default() },
                l1_data_gas_price: ResourcePrice { price_in_fri: Default::default(), price_in_wei: Default::default() },
                l1_da_mode: L1DataAvailabilityMode::Blob,
                starknet_version: starknet_version.to_string(),
                event_commitment: Default::default(),
                transaction_commitment: Default::default(),
                receipt_commitment: Default::default(),
                state_diff_commitment: Default::default(),
                event_count: 0,
                transaction_count: 0,
                state_diff_length: 0,
                transactions: vec![],
            })
            .unwrap()
        }
    }

    mod check_block_sync_tests {
        use super::*;

        #[test]
        fn test_batch_closed_returns_batch_closed() {
            let batch = create_test_batch(
                5,
                AggregatorBatchStatus::Closed,
                StarknetVersion::V0_14_2,
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
                StarknetVersion::V0_14_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            // Block has different version than batch
            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_3, &limits);

            assert_eq!(result, BatchCheckResult::StarknetVersionMismatch);
        }

        #[test]
        fn test_orchestrator_version_mismatch_returns_mismatch() {
            // Create a batch with a different orchestrator version
            let batch = create_test_batch_with_orchestrator_version(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
                "different-version".to_string(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            // Even with matching Starknet version, orchestrator version mismatch should be detected
            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_2, &limits);

            assert_eq!(result, BatchCheckResult::OrchestratorVersionMismatch);
        }

        #[test]
        fn test_orchestrator_version_mismatch_checked_before_starknet_version() {
            // When both orchestrator version and starknet version mismatch,
            // orchestrator version mismatch should be returned first
            let batch = create_test_batch_with_orchestrator_version(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
                "different-version".to_string(),
            );
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_weights = AggregatorBatchWeights::new(100, 100);

            // Both versions mismatch, but orchestrator version is checked first
            let result = state.check_block_sync(&block_weights, StarknetVersion::V0_13_3, &limits);

            assert_eq!(result, BatchCheckResult::OrchestratorVersionMismatch);
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

        #[tokio::test]
        async fn test_checked_add_block_closes_when_accumulated_aggregator_input_size_exceeds_limit() {
            let mut batch = create_test_batch(
                5,
                AggregatorBatchStatus::Open,
                StarknetVersion::V0_13_2,
                AggregatorBatchWeights::new(100, 100),
                Utc::now(),
            );
            batch.aggregator_input_size_upper_bound = TEST_MAX_AGGREGATOR_INPUT_SIZE - 10;
            let state = NonEmptyAggregatorState::new(batch, create_test_state_update());
            let limits = create_test_limits();
            let block_state_update = create_test_state_update();
            let block_weights = AggregatorBatchWeights::new(100, 0);
            let batch_client = BatchRpcClient::with_defaults(url::Url::parse("http://localhost:0").unwrap());

            let result = state
                .checked_add_block_with_limits(
                    6,
                    &block_state_update,
                    &block_weights,
                    StarknetVersion::V0_14_2,
                    &limits,
                    &batch_client,
                )
                .await
                .unwrap();

            assert!(result.is_none());
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
                            BatchCheckResult::StarknetVersionMismatch,
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
            let mut state = NonEmptyAggregatorState::new(batch, create_test_state_update());

            state.close();

            assert_eq!(state.batch.status, AggregatorBatchStatus::Closed);
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
            let mut state = NonEmptyAggregatorState::new(batch, create_test_state_update());

            state.close();

            assert_eq!(state.batch.index, original_index);
            assert_eq!(state.batch.start_block, original_start_block);
            assert_eq!(state.batch.end_block, original_end_block);
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
            let limits = AggregatorBatchConfig::new_for_test(
                5000,
                20,
                AggregatorBatchWeights::new(100, 200),
                7200,
                100,
                750_000,
            );

            assert_eq!(limits.max_blob_size, 5000);
            assert_eq!(limits.max_batch_size, 20);
            assert_eq!(limits.max_batch_builtin_weights.l1_gas, 100);
            assert_eq!(limits.max_batch_builtin_weights.message_segment_length, 200);
            assert_eq!(limits.max_batch_time_seconds, 7200);
            assert_eq!(limits.empty_block_proving_gas, 100);
            assert_eq!(limits.max_aggregator_input_size, 750_000);
        }
    }
}
