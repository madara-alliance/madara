use crate::core::config::{Config, ConfigParam, StarknetVersion};
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{SnosBatch, SnosBatchStatus, SnosBatchUpdates};
use crate::worker::event_handler::triggers::batching_v2::utils::get_block_builtin_weights;
use crate::worker::event_handler::triggers::batching_v2::BlockProcessingResult;
use crate::worker::event_handler::triggers::snos::fetch_block_starknet_version;
use blockifier::bouncer::BouncerWeights;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use std::sync::Arc;
use tracing::{debug, info};

pub enum SnosState {
    Empty(EmptySnosState),
    NonEmpty(NonEmptySnosState),
}

pub struct EmptySnosState {
    index: u64,
}

pub struct NonEmptySnosState {
    pub batch: SnosBatch,
}

pub struct SnosStateHandler {
    config: Arc<Config>,
}

impl SnosStateHandler {
    #[allow(dead_code)]
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub fn from_config(config: &Arc<Config>) -> Self {
        Self { config: config.clone() }
    }

    /// Load the latest SNOS batch state from the database
    ///
    /// If a batch exists and is open, we return it.
    /// If the batch is closed, we return an empty state.
    pub async fn load_batch_state(&self) -> Result<SnosState, JobError> {
        let batch = self.config.database().get_latest_snos_batch().await?;

        if let Some(batch) = batch {
            if batch.status.is_closed() {
                // Batch is closed, return empty state
                Ok(SnosState::Empty(EmptySnosState::new(batch.index + 1)))
            } else {
                Ok(SnosState::NonEmpty(NonEmptySnosState::new(batch)))
            }
        } else {
            Ok(SnosState::Empty(EmptySnosState::new(1)))
        }
    }

    /// Save the given SNOS batch state to the database
    ///
    /// Updates or creates the batch document in the DB
    pub async fn save_batch_state(&self, state: &NonEmptySnosState) -> Result<(), JobError> {
        info!(batch=?state.batch, "Saving snos state");
        self.config.database().update_or_create_snos_batch(&state.batch, &SnosBatchUpdates::default()).await?;
        Ok(())
    }
}

pub struct SnosBatchLimits {
    /// Maximum number of blocks per SNOS batch (if configured)
    pub max_batch_size: u64,
    /// Fixed number of blocks per SNOS batch (for testing, if configured)
    pub fixed_batch_size: Option<u64>,
    /// Maximum time a batch can remain open in seconds
    pub max_batch_time_seconds: u64,
    /// Maximum bouncer weights limit for a SNOS batch
    pub max_bouncer_weights: BouncerWeights,
}

impl SnosBatchLimits {
    pub fn from_config(config: &ConfigParam) -> Self {
        Self {
            max_batch_size: config.batching_config.max_blocks_per_snos_batch,
            fixed_batch_size: config.batching_config.fixed_blocks_per_snos_batch,
            max_batch_time_seconds: config.batching_config.max_batch_time_seconds,
            max_bouncer_weights: config.bouncer_weights_limit,
        }
    }
}

pub struct SnosHandler {
    config: Arc<Config>,
    limits: SnosBatchLimits,
    empty_block_proving_gas: u64,
}

impl SnosHandler {
    pub fn new(config: Arc<Config>, limits: SnosBatchLimits, empty_block_proving_gas: u64) -> Self {
        Self { config, limits, empty_block_proving_gas }
    }

    /// Include a block in SNOS batching
    ///
    /// This is the main entry point for processing a block. It handles both empty and non-empty states.
    /// TODO: For empty state, the caller should determine the next batch index to use.
    /// This method is typically called from the batching trigger which manages state properly.
    pub async fn include_block(
        &self,
        block_num: u64,
        aggregator_batch_index: Option<u64>,
        state: SnosState,
    ) -> Result<BlockProcessingResult<SnosState>, JobError> {
        info!("Including block {} in snos batch", block_num);
        match state {
            SnosState::Empty(ref empty_state) => {
                // Get the starknet version of the current block
                let block_version =
                    fetch_block_starknet_version(self.config.madara_rpc_client(), block_num).await.map_err(|e| {
                        JobError::Other(OtherError(eyre!(
                            "Failed to fetch Starknet version for block {}: {}",
                            block_num,
                            e
                        )))
                    })?;
                let block_weights = get_block_builtin_weights(
                    block_num,
                    self.config.madara_feeder_gateway_client(),
                    self.empty_block_proving_gas,
                )
                .await?;
                let new_state = self
                    .start_snos_batch(
                        empty_state.index,
                        aggregator_batch_index,
                        block_num,
                        block_weights,
                        block_version,
                    )
                    .await?;
                Ok(BlockProcessingResult::Accumulated(SnosState::NonEmpty(new_state)))
            }
            SnosState::NonEmpty(state) => {
                // Process the block with the existing state
                self.process_block(block_num, aggregator_batch_index, state).await
            }
        }
    }

    /// Process a single block and update the SNOS batch state
    ///
    /// This method assumes:
    /// 1. For L2s: The block has already been assigned to an aggregator batch
    /// 2. Starknet version checks have been done during aggregator batching (for L2s)
    /// 3. We're operating on a valid range calculated externally
    ///
    /// Returns:
    /// - Accumulated: Block added to current batch
    /// - BatchCompleted: Current batch is full, new batch started with this block
    pub async fn process_block(
        &self,
        block_num: u64,
        aggregator_batch_index: Option<u64>,
        state: NonEmptySnosState,
    ) -> Result<BlockProcessingResult<SnosState>, JobError> {
        // Get the bouncer weights for the current block
        let block_weights = get_block_builtin_weights(
            block_num,
            self.config.madara_feeder_gateway_client(),
            self.empty_block_proving_gas,
        )
        .await?;
        // Get the starknet version of the current block
        let block_version =
            fetch_block_starknet_version(self.config.madara_rpc_client(), block_num).await.map_err(|e| {
                JobError::Other(OtherError(eyre!("Failed to fetch Starknet version for block {}: {}", block_num, e)))
            })?;

        // TODO: We can send the aggregator batch index in the argument here
        match state
            .checked_add_block_with_limits(
                block_num,
                block_weights,
                block_version,
                aggregator_batch_index,
                &self.limits,
            )
            .await?
        {
            Some(updated_state) => Ok(BlockProcessingResult::Accumulated(SnosState::NonEmpty(updated_state))),
            None => {
                info!(
                    snos_batch_index = %state.batch.index,
                    num_blocks = %state.batch.num_blocks,
                    end_block = %state.batch.end_block,
                    next_block = %block_num,
                    "Weights addition failed, closing SNOS batch and starting new batch"
                );

                let completed_state = SnosState::NonEmpty(state.close());
                let new_state = SnosState::NonEmpty(
                    self.start_snos_batch(
                        state.batch.index + 1,
                        aggregator_batch_index,
                        block_num,
                        block_weights,
                        block_version,
                    )
                    .await?,
                );

                Ok(BlockProcessingResult::BatchCompleted { completed_state, new_state })
            }
        }
    }

    /// Start a new SNOS batch with the given parameters
    pub async fn start_snos_batch(
        &self,
        index: u64,
        aggregator_batch_index: Option<u64>,
        start_block: u64,
        initial_weights: BouncerWeights,
        version: StarknetVersion,
    ) -> Result<NonEmptySnosState, JobError> {
        let batch = SnosBatch::new(index, aggregator_batch_index, start_block, initial_weights, version);
        Ok(NonEmptySnosState::new(batch))
    }
}

impl EmptySnosState {
    pub fn new(index: u64) -> Self {
        EmptySnosState { index }
    }
}

impl NonEmptySnosState {
    pub fn new(batch: SnosBatch) -> Self {
        Self { batch }
    }

    pub async fn checked_add_block_with_limits(
        &self,
        block_num: u64,
        block_weights: BouncerWeights,
        block_version: StarknetVersion,
        block_aggregator_batch_index: Option<u64>,
        batch_limits: &SnosBatchLimits,
    ) -> Result<Option<Self>, JobError> {
        // If a fixed size is set, use only that to decide if we should close the SNOS batch
        if let Some(fixed_blocks_per_snos_batch) = batch_limits.fixed_batch_size {
            // If the MADARA_ORCHESTRATOR_FIXED_BLOCKS_PER_SNOS_BATCH env is set, we use that value
            // Mostly, it'll be used for testing purposes
            debug!("Using MADARA_ORCHESTRATOR_FIXED_BLOCKS_PER_SNOS_BATCH env variable to close snos batch with max blocks = {}", fixed_blocks_per_snos_batch);
            return Ok(None);
        }

        if self.batch.status.is_closed()
            || block_version != self.batch.starknet_version
            || self.batch.num_blocks >= batch_limits.max_batch_size
            || (Utc::now().round_subsecs(0) - self.batch.created_at).abs().num_seconds() as u64
                >= batch_limits.max_batch_time_seconds
        {
            return Ok(None);
        }

        if let Some(snos_batch_aggregator_batch_index) = self.batch.aggregator_batch_index {
            match block_aggregator_batch_index {
                None => {
                    // This is not expected
                    return Err(JobError::Other(OtherError(eyre!(
                        "Aggregator batch not found for block {} during SNOS batching. This should not happen!",
                        block_num
                    ))));
                }
                Some(block_aggregator_batch_index) => {
                    if snos_batch_aggregator_batch_index == block_aggregator_batch_index {
                        return Ok(None);
                    }
                }
            }
        }

        // Check if adding the next block would overflow the bouncer weights
        let combined_weights = match self.batch.builtin_weights.checked_add(block_weights) {
            Some(combined_weights) => {
                // Check if combined weights exceed the limit
                if batch_limits.max_bouncer_weights.checked_sub(combined_weights).is_none() {
                    return Ok(None);
                } else {
                    combined_weights
                }
            }
            None => {
                return Ok(None);
            }
        };

        Ok(Some(NonEmptySnosState::new(self.batch.update(block_num, combined_weights, None))))
    }

    /// Try to add a block to the current batch
    ///
    /// Returns Some(updated_state) if the block was added successfully,
    /// None if weights would overflow
    #[allow(dead_code)]
    pub fn checked_add_block(&self, block_num: u64, block_weights: BouncerWeights) -> Option<Self> {
        // Try to add the weights
        let new_builtin_weights = self.batch.builtin_weights.checked_add(block_weights)?;

        // Create updated batch
        let mut updated_batch = self.batch.clone();
        updated_batch.end_block = block_num;
        updated_batch.num_blocks = block_num - updated_batch.start_block + 1;
        updated_batch.builtin_weights = new_builtin_weights;
        updated_batch.updated_at = Utc::now().round_subsecs(0);

        Some(Self { batch: updated_batch })
    }

    /// Close the current batch
    ///
    /// Returns a new state with the batch status set to Closed
    pub fn close(&self) -> Self {
        let mut closed_batch = self.batch.clone();
        closed_batch.status = SnosBatchStatus::Closed;
        closed_batch.updated_at = Utc::now().round_subsecs(0);

        Self { batch: closed_batch }
    }
}
