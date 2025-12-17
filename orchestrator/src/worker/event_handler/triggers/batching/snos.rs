use crate::core::config::{Config, ConfigParam, StarknetVersion};
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::batch::{SnosBatch, SnosBatchStatus, SnosBatchUpdates};
use crate::worker::event_handler::triggers::batching::utils::{get_block_builtin_weights, get_block_version};
use crate::worker::event_handler::triggers::batching::BlockProcessingResult;
use blockifier::bouncer::BouncerWeights;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::eyre;
use std::sync::Arc;
use tracing::{debug, info, warn};

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
        info!(batch=?state.batch, "Saving snos batch state");
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

    /// Create limits for testing purposes
    #[cfg(test)]
    pub fn new_for_test(
        max_batch_size: u64,
        fixed_batch_size: Option<u64>,
        max_batch_time_seconds: u64,
        max_bouncer_weights: BouncerWeights,
    ) -> Self {
        Self { max_batch_size, fixed_batch_size, max_batch_time_seconds, max_bouncer_weights }
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
    /// This method is typically called from the batching trigger which manages state properly.
    pub async fn include_block(
        &self,
        block_num: u64,
        aggregator_batch_index: Option<u64>,
        state: SnosState,
    ) -> Result<BlockProcessingResult<SnosState, NonEmptySnosState>, JobError> {
        info!("Including block {} in snos batch", block_num);
        match state {
            SnosState::Empty(ref empty_state) => {
                // Get the starknet version of the current block
                let block_version = get_block_version(block_num, self.config.madara_rpc_client()).await?;
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
                Ok(BlockProcessingResult::Accumulated(new_state))
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
    ) -> Result<BlockProcessingResult<SnosState, NonEmptySnosState>, JobError> {
        // Get the bouncer weights for the current block
        let block_weights = get_block_builtin_weights(
            block_num,
            self.config.madara_feeder_gateway_client(),
            self.empty_block_proving_gas,
        )
        .await?;
        // Get the starknet version of the current block
        let block_version = get_block_version(block_num, self.config.madara_rpc_client()).await?;

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
            Some(updated_state) => Ok(BlockProcessingResult::Accumulated(updated_state)),
            None => {
                info!(
                    snos_batch_index = %state.batch.index,
                    num_blocks = %state.batch.num_blocks,
                    end_block = %state.batch.end_block,
                    next_block = %block_num,
                    "Weights addition failed, closing SNOS batch and starting new batch"
                );

                let completed_state = state.close();
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

/// Result of checking if a block can be added to a SNOS batch
#[derive(Debug, PartialEq)]
pub enum SnosBatchCheckResult {
    /// Block can be added, here are the combined weights
    CanAdd(BouncerWeights),
    /// Block can be added with fixed batch size (no weight accumulation)
    CanAddFixedSize,
    /// Fixed batch size reached
    FixedSizeReached,
    /// Batch is already closed
    BatchClosed,
    /// Starknet version mismatch
    VersionMismatch,
    /// Max blocks per batch reached
    MaxBlocksReached,
    /// Batch time limit exceeded
    TimeLimitExceeded,
    /// Aggregator batch index mismatch (L2 only)
    AggregatorBatchMismatch,
    /// Missing aggregator batch index when expected (error case)
    MissingAggregatorBatch,
    /// Weight overflow during addition
    WeightOverflow,
    /// Combined weights exceed the limit
    WeightLimitExceeded,
}

impl NonEmptySnosState {
    pub fn new(batch: SnosBatch) -> Self {
        Self { batch }
    }

    /// Check if a block can be added to this batch based on synchronous conditions.
    ///
    /// Returns `SnosBatchCheckResult::CanAdd(combined_weights)` if all checks pass,
    /// or a specific reason why the block cannot be added.
    pub fn check_block_sync(
        &self,
        block_weights: BouncerWeights,
        block_version: StarknetVersion,
        block_aggregator_batch_index: Option<u64>,
        batch_limits: &SnosBatchLimits,
    ) -> SnosBatchCheckResult {
        // If a fixed size is set, use only that to decide if we should close the SNOS batch
        if let Some(fixed_blocks_per_snos_batch) = batch_limits.fixed_batch_size {
            return if self.batch.num_blocks >= fixed_blocks_per_snos_batch {
                SnosBatchCheckResult::FixedSizeReached
            } else {
                SnosBatchCheckResult::CanAddFixedSize
            };
        }

        // Check if batch is already closed
        if self.batch.status.is_closed() {
            return SnosBatchCheckResult::BatchClosed;
        }

        // Check version mismatch
        if block_version != self.batch.starknet_version {
            return SnosBatchCheckResult::VersionMismatch;
        }

        // Check max blocks reached
        if self.batch.num_blocks >= batch_limits.max_batch_size {
            return SnosBatchCheckResult::MaxBlocksReached;
        }

        // Check time limit
        let elapsed_seconds = (Utc::now().round_subsecs(0) - self.batch.created_at).abs().num_seconds() as u64;
        if elapsed_seconds >= batch_limits.max_batch_time_seconds {
            return SnosBatchCheckResult::TimeLimitExceeded;
        }

        // Check aggregator batch index (for L2)
        if let Some(snos_batch_aggregator_batch_index) = self.batch.aggregator_batch_index {
            match block_aggregator_batch_index {
                None => {
                    return SnosBatchCheckResult::MissingAggregatorBatch;
                }
                Some(block_agg_index) => {
                    if snos_batch_aggregator_batch_index != block_agg_index {
                        return SnosBatchCheckResult::AggregatorBatchMismatch;
                    }
                }
            }
        }

        // Check weight overflow and limit
        match self.batch.builtin_weights.checked_add(block_weights) {
            Some(combined_weights) => {
                if batch_limits.max_bouncer_weights.checked_sub(combined_weights).is_none() {
                    SnosBatchCheckResult::WeightLimitExceeded
                } else {
                    SnosBatchCheckResult::CanAdd(combined_weights)
                }
            }
            None => SnosBatchCheckResult::WeightOverflow,
        }
    }

    pub async fn checked_add_block_with_limits(
        &self,
        block_num: u64,
        block_weights: BouncerWeights,
        block_version: StarknetVersion,
        block_aggregator_batch_index: Option<u64>,
        batch_limits: &SnosBatchLimits,
    ) -> Result<Option<Self>, JobError> {
        let check_result =
            self.check_block_sync(block_weights, block_version, block_aggregator_batch_index, batch_limits);
        match check_result {
            SnosBatchCheckResult::CanAdd(combined_weights) => {
                Ok(Some(NonEmptySnosState::new(self.batch.update(block_num, combined_weights, None))))
            }
            SnosBatchCheckResult::CanAddFixedSize => {
                warn!(
                    "Using MADARA_ORCHESTRATOR_FIXED_BLOCKS_PER_SNOS_BATCH env variable to close snos batch with max blocks = {:?}",
                    batch_limits.fixed_batch_size
                );
                Ok(Some(NonEmptySnosState::new(self.batch.update(block_num, BouncerWeights::empty(), None))))
            }
            SnosBatchCheckResult::FixedSizeReached => {
                debug!(
                    batch_index = %self.batch.index,
                    block_num = %block_num,
                    fixed_batch_size = ?batch_limits.fixed_batch_size,
                    "Closing SNOS batch: FixedSizeReached"
                );
                Ok(None)
            }
            SnosBatchCheckResult::MissingAggregatorBatch => Err(JobError::Other(OtherError(eyre!(
                "Aggregator batch not found for block {} during SNOS batching. This should not happen!",
                block_num
            )))),
            reason => {
                debug!(
                    batch_index = %self.batch.index,
                    block_num = %block_num,
                    reason = ?reason,
                    "Closing SNOS batch"
                );
                Ok(None)
            }
        }
    }

    /// Close the current batch
    ///
    /// Returns a new state with the batch status set to Closed
    pub fn close(&self) -> Self {
        let mut closed_batch = self.batch.clone();
        closed_batch.status = SnosBatchStatus::Closed;
        Self { batch: closed_batch }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    /// Helper to create a test SNOS batch with configurable parameters
    fn create_test_snos_batch(
        num_blocks: u64,
        status: SnosBatchStatus,
        version: StarknetVersion,
        weights: BouncerWeights,
        aggregator_batch_index: Option<u64>,
        created_at: chrono::DateTime<Utc>,
    ) -> SnosBatch {
        SnosBatch {
            id: uuid::Uuid::new_v4(),
            index: 1,
            aggregator_batch_index,
            starknet_version: version,
            start_block: 0,
            end_block: num_blocks.saturating_sub(1),
            num_blocks,
            builtin_weights: weights,
            status,
            created_at,
            updated_at: Utc::now(),
        }
    }

    /// Helper to create test weights with all fields set
    fn create_test_weights(l1_gas: usize, msg_len: usize) -> BouncerWeights {
        use starknet_api::execution_resources::GasAmount;
        BouncerWeights {
            l1_gas,
            message_segment_length: msg_len,
            n_events: 0,
            state_diff_size: 0,
            sierra_gas: GasAmount(0),
            n_txs: 0,
            proving_gas: GasAmount(0),
        }
    }

    /// Helper to create default test limits with generous maximums
    fn create_test_limits() -> SnosBatchLimits {
        use starknet_api::execution_resources::GasAmount;
        SnosBatchLimits::new_for_test(
            10,   // max_batch_size (10 blocks)
            None, // no fixed batch size
            3600, // max_batch_time_seconds (1 hour)
            BouncerWeights {
                l1_gas: 1_000_000,
                message_segment_length: 10000,
                n_events: 1_000_000,
                state_diff_size: 1_000_000,
                sierra_gas: GasAmount(1_000_000_000),
                n_txs: 1_000_000,
                proving_gas: GasAmount(1_000_000_000),
            },
        )
    }

    mod check_block_sync_tests {
        use super::*;

        #[test]
        fn test_fixed_batch_size_reached() {
            let batch = create_test_snos_batch(
                5, // 5 blocks already
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = SnosBatchLimits::new_for_test(
                10,
                Some(5), // fixed batch size of 5
                3600,
                create_test_weights(1_000_000, 10000),
            );
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::FixedSizeReached);
        }

        #[test]
        fn test_fixed_batch_size_can_add() {
            let batch = create_test_snos_batch(
                4, // 4 blocks, one less than fixed size
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = SnosBatchLimits::new_for_test(
                10,
                Some(5), // fixed batch size of 5
                3600,
                create_test_weights(1_000_000, 10000),
            );
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::CanAddFixedSize);
        }

        #[test]
        fn test_batch_closed_returns_batch_closed() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Closed,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::BatchClosed);
        }

        #[test]
        fn test_version_mismatch_returns_version_mismatch() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            // Block has different version than batch
            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_3, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::VersionMismatch);
        }

        #[test]
        fn test_max_blocks_reached_returns_max_blocks_reached() {
            let batch = create_test_snos_batch(
                10, // Already at max (limit is 10)
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::MaxBlocksReached);
        }

        #[test]
        fn test_time_limit_exceeded_returns_time_limit_exceeded() {
            let old_time = Utc::now() - Duration::seconds(3700); // More than 1 hour ago
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                old_time,
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::TimeLimitExceeded);
        }

        #[test]
        fn test_aggregator_batch_mismatch_returns_mismatch() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                Some(1), // SNOS batch is linked to aggregator batch 1
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            // Block belongs to aggregator batch 2
            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, Some(2), &limits);

            assert_eq!(result, SnosBatchCheckResult::AggregatorBatchMismatch);
        }

        #[test]
        fn test_missing_aggregator_batch_when_expected() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                Some(1), // SNOS batch expects aggregator batch
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            // Block has no aggregator batch index
            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::MissingAggregatorBatch);
        }

        #[test]
        fn test_weight_overflow_returns_weight_overflow() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(usize::MAX, 100), // Near overflow
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(1, 0); // Will overflow

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::WeightOverflow);
        }

        #[test]
        fn test_weight_limit_exceeded_returns_weight_limit_exceeded() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(900_000, 9000), // Near limit
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits(); // limit is 1_000_000, 10000
            let block_weights = create_test_weights(200_000, 2000); // Would exceed limit

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::WeightLimitExceeded);
        }

        #[test]
        fn test_all_conditions_pass_returns_can_add() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100_000, 1000),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(50_000, 500);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            // Should return CanAdd with combined weights
            match result {
                SnosBatchCheckResult::CanAdd(combined) => {
                    assert_eq!(combined.l1_gas, 150_000);
                    assert_eq!(combined.message_segment_length, 1500);
                }
                other => panic!("Expected CanAdd, got {:?}", other),
            }
        }

        #[test]
        fn test_l3_mode_no_aggregator_constraint() {
            // L3 batches don't have aggregator_batch_index set (None)
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None, // L3 mode - no aggregator batch
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            // Even if block has no aggregator batch index, it should work
            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert!(matches!(result, SnosBatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_l2_mode_matching_aggregator_batch() {
            // L2 batches have aggregator_batch_index set
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                Some(1), // L2 mode - linked to aggregator batch 1
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            // Block also belongs to aggregator batch 1
            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, Some(1), &limits);

            assert!(matches!(result, SnosBatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_exactly_at_max_blocks_minus_one_can_add() {
            let batch = create_test_snos_batch(
                9, // One less than max
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert!(matches!(result, SnosBatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_exactly_at_weight_limit_can_add() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(500_000, 5000),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            // Combined will be exactly at limit (1_000_000, 10000)
            let block_weights = create_test_weights(500_000, 5000);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert!(matches!(result, SnosBatchCheckResult::CanAdd(_)));
        }

        #[test]
        fn test_snos_job_created_status_treated_as_closed() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::SnosJobCreated,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);
            let limits = create_test_limits();
            let block_weights = create_test_weights(100, 100);

            let result = state.check_block_sync(block_weights, StarknetVersion::V0_13_2, None, &limits);

            assert_eq!(result, SnosBatchCheckResult::BatchClosed);
        }

        #[test]
        fn test_multiple_version_transitions() {
            let versions = [StarknetVersion::V0_13_2, StarknetVersion::V0_13_3, StarknetVersion::V0_13_4];

            for batch_version in &versions {
                for block_version in &versions {
                    let batch = create_test_snos_batch(
                        5,
                        SnosBatchStatus::Open,
                        *batch_version,
                        create_test_weights(100, 100),
                        None,
                        Utc::now(),
                    );
                    let state = NonEmptySnosState::new(batch);
                    let limits = create_test_limits();
                    let block_weights = create_test_weights(100, 100);

                    let result = state.check_block_sync(block_weights, *block_version, None, &limits);

                    if batch_version == block_version {
                        assert!(
                            matches!(result, SnosBatchCheckResult::CanAdd(_)),
                            "Same version {:?} should allow add",
                            batch_version
                        );
                    } else {
                        assert_eq!(
                            result,
                            SnosBatchCheckResult::VersionMismatch,
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
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                None,
                Utc::now(),
            );
            let state = NonEmptySnosState::new(batch);

            let closed_state = state.close();

            assert_eq!(closed_state.batch.status, SnosBatchStatus::Closed);
        }

        #[test]
        fn test_close_preserves_other_fields() {
            let batch = create_test_snos_batch(
                5,
                SnosBatchStatus::Open,
                StarknetVersion::V0_13_2,
                create_test_weights(100, 100),
                Some(1),
                Utc::now(),
            );
            let original_index = batch.index;
            let original_start_block = batch.start_block;
            let original_aggregator_batch_index = batch.aggregator_batch_index;
            let state = NonEmptySnosState::new(batch);

            let closed_state = state.close();

            assert_eq!(closed_state.batch.index, original_index);
            assert_eq!(closed_state.batch.start_block, original_start_block);
            assert_eq!(closed_state.batch.aggregator_batch_index, original_aggregator_batch_index);
        }
    }

    mod empty_state_tests {
        use super::*;

        #[test]
        fn test_empty_state_new() {
            let state = EmptySnosState::new(5);
            assert_eq!(state.index, 5);
        }
    }

    mod limits_tests {
        use super::*;

        #[test]
        fn test_limits_new_for_test() {
            let limits = SnosBatchLimits::new_for_test(20, Some(10), 7200, create_test_weights(100, 200));

            assert_eq!(limits.max_batch_size, 20);
            assert_eq!(limits.fixed_batch_size, Some(10));
            assert_eq!(limits.max_batch_time_seconds, 7200);
            assert_eq!(limits.max_bouncer_weights.l1_gas, 100);
            assert_eq!(limits.max_bouncer_weights.message_segment_length, 200);
        }

        #[test]
        fn test_limits_without_fixed_batch_size() {
            let limits = SnosBatchLimits::new_for_test(20, None, 7200, create_test_weights(100, 200));

            assert_eq!(limits.fixed_batch_size, None);
        }
    }
}
