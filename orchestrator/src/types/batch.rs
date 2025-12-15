use crate::core::config::StarknetVersion;
use crate::types::constant::{STORAGE_BLOB_DIR, STORAGE_STATE_UPDATE_DIR};
use blockifier::bouncer::BouncerWeights;
use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::{chrono_datetime_as_bson_datetime, uuid_1_as_binary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status enum for Aggregator batches
///
/// Represents the various states an aggregator batch can be in during its lifecycle.
/// The status flows generally follow this pattern:
/// Open -> Closed -> PendingAggregatorRun -> PendingAggregatorVerification -> ReadyForStateUpdate -> Completed
///
/// Error states can occur at any point and include: BatchCreationFailed, AggregationFailed,
/// VerificationFailed, StateUpdateFailed
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, strum_macros::Display, Eq, Default)]
pub enum AggregatorBatchStatus {
    /// Batch is open and new blocks can be added to it
    #[default]
    Open,
    /// Batch is closed and no new blocks can be added to it
    Closed,
    /// Batch can be processed by the aggregator job
    /// This means that all the child jobs completed by the prover client, and we can close the bucket
    PendingAggregatorRun,
    /// Batch is closed, and we are waiting for SUCCESS from the prover client for the bucket ID
    PendingAggregatorVerification,
    /// Bucket is verified and is ready for state update
    ReadyForStateUpdate,
    /// Batch processing is complete and state update is done
    Completed,
    /// Batch creation failed
    BatchCreationFailed,
    /// Aggregator job failed
    AggregationFailed,
    /// Verification job failed
    VerificationFailed,
    /// State update failed
    StateUpdateFailed,
}

impl AggregatorBatchStatus {
    pub fn is_closed(&self) -> bool {
        !matches!(self, AggregatorBatchStatus::Open)
    }
}

/// Update structure for Aggregator batches
///
/// Contains optional fields that can be updated for an aggregator batch.
/// Used in database update operations to specify which fields should be modified.
#[derive(Default, Serialize, Debug, Clone)]
pub struct AggregatorBatchUpdates {
    /// Updated end block number
    pub end_block: Option<u64>,
    /// Updated batch status
    pub status: Option<AggregatorBatchStatus>,
}

/// Update structure for SNOS batches
///
/// Contains optional fields that can be updated for a SNOS batch.
/// Used in database update operations to specify which fields should be modified.
#[derive(Default, Serialize, Debug, Clone)]
pub struct SnosBatchUpdates {
    /// Updated end block number
    pub end_block: Option<u64>,
    /// Updated batch status
    pub status: Option<SnosBatchStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AggregatorBatchWeights {
    pub l1_gas: usize,
    pub message_segment_length: usize,
}

/// Aggregator Batch
///
/// Represents a high-level batch that contains multiple SNOS batches and manages
/// the aggregation process for proof generation and settlement. Aggregator batches
/// are the primary unit for managing the proving pipeline.
///
/// # Lifecycle
/// 1. **Open**: Batch is created and can accept new blocks/SNOS batches
/// 2. **Closed**: Batch is full and ready for processing
/// 3. **Processing**: Goes through aggregation, verification, and settlement
/// 4. **Completed**: All processing is done
///
/// # Relationship with SNOS Batches
/// - One aggregator batch contains multiple SNOS batches
/// - When an aggregator batch closes, all its SNOS batches are also closed
/// - SNOS batches cannot exist without a parent aggregator batch
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AggregatorBatch {
    // Statis fields - set when starting and don't change after that
    /// Unique identifier for the batch
    #[cfg_attr(feature = "with_mongodb", serde(rename = "_id", with = "uuid_1_as_binary"))]
    pub id: Uuid,

    /// Sequential index of the aggregator batch (unique, auto-incrementing)
    /// Used for ordering and referencing batches
    pub index: u64,

    /// Bucket ID for the batch, received from the prover client
    /// Used to track the batch in the proving system
    pub bucket_id: String,

    /// Path to the squashed state updates file
    /// This is done for optimization so we don't have to create a new squashed
    /// state update from scratch
    pub squashed_state_updates_path: String,

    /// Path to the compressed state update converted to a blob
    /// Used for data availability and storage optimization
    pub blob_path: String,

    /// Starknet protocol version for all blocks in this batch
    /// All blocks in a batch must have the same Starknet version for prover compatibility
    pub starknet_version: StarknetVersion,

    /// Start block number of the aggregator batch (inclusive)
    /// This is the lowest block number across all contained SNOS batches
    pub start_block: u64,

    // Dynamic fields - can change when adding a new block to the batch
    /// End block number of the aggregator batch (inclusive)
    /// This is the highest block number across all contained SNOS batches
    pub end_block: u64,

    /// Number of blocks covered by this aggregator batch
    /// This spans across all SNOS batches contained within this aggregator batch
    pub num_blocks: u64,

    /// Length of vector of felts representing the compressed blob data for the batch
    pub blob_len: usize,

    /// Builtin weights for the batch. We decide when to close a batch based on this.
    pub builtin_weights: AggregatorBatchWeights,

    /// Current status of the aggregator batch
    pub status: AggregatorBatchStatus,

    // Audit fields
    /// Timestamp when the batch was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,

    /// Timestamp when the batch was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
}

impl AggregatorBatchWeights {
    pub fn new(l1_gas: usize, message_segment_length: usize) -> Self {
        Self { l1_gas, message_segment_length }
    }

    pub fn checked_add(&self, other: &AggregatorBatchWeights) -> Option<AggregatorBatchWeights> {
        Some(Self {
            l1_gas: self.l1_gas.checked_add(other.l1_gas)?,
            message_segment_length: self.message_segment_length.checked_add(other.message_segment_length)?,
        })
    }

    pub fn checked_sub(&self, other: &AggregatorBatchWeights) -> Option<AggregatorBatchWeights> {
        Some(Self {
            l1_gas: self.l1_gas.checked_sub(other.l1_gas)?,
            message_segment_length: self.message_segment_length.checked_sub(other.message_segment_length)?,
        })
    }
}

impl From<&BouncerWeights> for AggregatorBatchWeights {
    fn from(weights: &BouncerWeights) -> Self {
        Self { l1_gas: weights.l1_gas, message_segment_length: weights.message_segment_length }
    }
}

impl AggregatorBatch {
    /// Creates a new aggregator batch
    ///
    /// # Arguments
    /// * `index` - Sequential index of the batch
    /// * `start_block` - Starting block number for the batch
    /// * `squashed_state_updates_path` - Path to store squashed state updates
    /// * `blob_path` - Path to store blob data
    /// * `bucket_id` - Identifier from the prover client
    ///
    /// # Returns
    /// A new `AggregatorBatch` instance with status `Open` and single block
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: u64,
        start_block: u64,
        bucket_id: String,
        blob_len: usize,
        builtin_weights: AggregatorBatchWeights,
        starknet_version: StarknetVersion,
    ) -> Self {
        // TODO: move get file path methods to this struct
        Self {
            id: Uuid::new_v4(),
            index,
            num_blocks: 1,
            start_block,
            end_block: start_block,
            squashed_state_updates_path: Self::get_state_update_file_path(index),
            blob_path: Self::get_blob_dir_path(index),
            blob_len,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
            bucket_id,
            starknet_version,
            status: AggregatorBatchStatus::Open,
            builtin_weights,
        }
    }

    pub fn update(
        &self,
        end_block: u64,
        blob_len: usize,
        weights: AggregatorBatchWeights,
        status: Option<AggregatorBatchStatus>,
    ) -> AggregatorBatch {
        let mut updated_batch = self.clone();
        updated_batch.end_block = end_block;
        updated_batch.num_blocks = end_block - updated_batch.start_block + 1;
        updated_batch.blob_len = blob_len;
        updated_batch.builtin_weights = weights;
        if let Some(status) = status {
            updated_batch.status = status;
        }
        updated_batch
    }

    pub fn get_blob_file_path(batch_index: u64, blob_index: u64) -> String {
        format!("{}/{}.txt", Self::get_blob_dir_path(batch_index), blob_index)
    }

    fn get_state_update_file_path(batch_index: u64) -> String {
        format!("{}/batch/{}.json", STORAGE_STATE_UPDATE_DIR, batch_index)
    }

    fn get_blob_dir_path(batch_index: u64) -> String {
        format!("{}/batch/{}", STORAGE_BLOB_DIR, batch_index)
    }
}

/// Status enum for SNOS batches
///
/// Represents the lifecycle states of a SNOS batch. SNOS batches have a simpler
/// lifecycle compared to aggregator batches.
#[derive(Serialize, Deserialize, Eq, PartialEq, strum_macros::Display, Debug, Clone, Default)]
pub enum SnosBatchStatus {
    /// Batch is open and new blocks can be added to it
    #[default]
    Open,
    /// Batch is closed and no new blocks can be added to it
    Closed,
    /// A SNOS job has been created for this batch
    SnosJobCreated,
    /// A SNOS job has beed Completed
    Completed,
}

impl SnosBatchStatus {
    pub fn is_closed(&self) -> bool {
        !matches!(self, SnosBatchStatus::Open)
    }
}

/// SNOS Batch
///
/// Represents a batch of blocks that will be processed by SNOS (Starknet OS).
/// SNOS batches are contained within aggregator batches and represent the
/// granular units for SNOS job processing.
///
/// # Lifecycle
/// 1. **Open**: Batch is created and can accept new blocks
/// 2. **Closed**: Batch is full or closed due to aggregator batch closure
/// 3. **SnosJobCreated**: A job has been created to process this batch
///
/// # Relationship with Aggregator Batches
/// - Each SNOS batch belongs to exactly one aggregator batch
/// - Multiple SNOS batches can exist within one aggregator batch
/// - SNOS batches are closed when their parent aggregator batch closes
/// - SNOS batches can also close independently based on their own criteria
#[derive(Serialize, Deserialize, PartialEq, Debug, Clone, Default)]
pub struct SnosBatch {
    // Statis fields - set when starting and don't change after that
    /// Unique identifier for the batch
    #[cfg_attr(feature = "with_mongodb", serde(rename = "_id", with = "uuid_1_as_binary"))]
    pub id: Uuid,

    /// SNOS batch sequential ID (unique across all SNOS batches, auto-incrementing)
    /// This provides a global ordering of SNOS batches independent of aggregator batches
    pub index: u64,

    /// Reference to the parent aggregator batch index
    /// This establishes the hierarchical relationship between SNOS and aggregator batches
    /// This is Optional since for L3s, we don't have aggregator batches
    pub aggregator_batch_index: Option<u64>,

    pub starknet_version: StarknetVersion,

    /// Start block number of the batch (inclusive)
    pub start_block: u64,

    // Dynamic fields - can change when adding a new block to the batch
    /// End block number of the batch (inclusive)
    pub end_block: u64,

    /// Number of blocks in this SNOS batch
    pub num_blocks: u64,

    /// Weights for this SNOS batch
    pub builtin_weights: BouncerWeights,

    /// Current status of the SNOS batch
    pub status: SnosBatchStatus,

    // Audit fields
    /// Timestamp when the batch was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,

    /// Timestamp when the batch was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
}

impl SnosBatch {
    /// Creates a new SNOS batch
    ///
    /// # Arguments
    /// * `snos_batch_id` - Unique sequential ID for this SNOS batch
    /// * `aggregator_batch_index` - Index of the parent aggregator batch
    /// * `start_block` - The start block number of the batch
    /// * `end_block` - The end block number of the batch
    ///
    /// # Returns
    /// A new `SnosBatch` instance with status `Open`
    ///
    /// # Panics
    ///
    /// Panics if `end_block` < `start_block`
    pub fn new(
        index: u64,
        aggregator_batch_index: Option<u64>,
        start_block: u64,
        builtin_weights: BouncerWeights,
        starknet_version: StarknetVersion,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            aggregator_batch_index,
            starknet_version,
            start_block,
            end_block: start_block,
            num_blocks: 1,
            status: SnosBatchStatus::Open,
            builtin_weights,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        }
    }

    pub fn update(&self, end_block: u64, weights: BouncerWeights, status: Option<SnosBatchStatus>) -> SnosBatch {
        let mut updated_batch = self.clone();
        updated_batch.end_block = end_block;
        updated_batch.num_blocks = end_block - updated_batch.start_block + 1;
        updated_batch.builtin_weights = weights;
        if let Some(status) = status {
            updated_batch.status = status;
        }
        updated_batch
    }

    /// Validates the internal consistency of the SNOS batch
    ///
    /// # Returns
    /// `Ok(())` if the batch is valid, `Err(String)` with error message if invalid
    pub fn validate(&self) -> Result<(), String> {
        if self.start_block > self.end_block {
            return Err("Start block cannot be greater than end block".to_string());
        }

        let expected_num_blocks = self.end_block - self.start_block + 1;
        if self.num_blocks != expected_num_blocks {
            return Err(format!("Inconsistent block count: expected {}, got {}", expected_num_blocks, self.num_blocks));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod aggregator_batch_weights_tests {
        use super::*;

        #[test]
        fn test_new() {
            let weights = AggregatorBatchWeights::new(1000, 500);
            assert_eq!(weights.l1_gas, 1000);
            assert_eq!(weights.message_segment_length, 500);
        }

        #[test]
        fn test_checked_add_success() {
            let weights1 = AggregatorBatchWeights::new(1000, 500);
            let weights2 = AggregatorBatchWeights::new(2000, 300);

            let result = weights1.checked_add(&weights2);
            assert!(result.is_some());

            let sum = result.unwrap();
            assert_eq!(sum.l1_gas, 3000);
            assert_eq!(sum.message_segment_length, 800);
        }

        #[test]
        fn test_checked_add_overflow_l1_gas() {
            let weights1 = AggregatorBatchWeights::new(usize::MAX, 100);
            let weights2 = AggregatorBatchWeights::new(1, 100);

            let result = weights1.checked_add(&weights2);
            assert!(result.is_none());
        }

        #[test]
        fn test_checked_add_overflow_message_segment_length() {
            let weights1 = AggregatorBatchWeights::new(100, usize::MAX);
            let weights2 = AggregatorBatchWeights::new(100, 1);

            let result = weights1.checked_add(&weights2);
            assert!(result.is_none());
        }

        #[test]
        fn test_checked_add_max_values() {
            let weights1 = AggregatorBatchWeights::new(usize::MAX / 2, usize::MAX / 2);
            let weights2 = AggregatorBatchWeights::new(usize::MAX / 2, usize::MAX / 2);

            let result = weights1.checked_add(&weights2);
            assert!(result.is_some());

            let sum = result.unwrap();
            assert_eq!(sum.l1_gas, usize::MAX - 1);
            assert_eq!(sum.message_segment_length, usize::MAX - 1);
        }

        #[test]
        fn test_checked_sub_success() {
            let weights1 = AggregatorBatchWeights::new(2000, 500);
            let weights2 = AggregatorBatchWeights::new(1000, 300);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_some());

            let diff = result.unwrap();
            assert_eq!(diff.l1_gas, 1000);
            assert_eq!(diff.message_segment_length, 200);
        }

        #[test]
        fn test_checked_sub_with_zero() {
            let weights1 = AggregatorBatchWeights::new(1000, 500);
            let weights2 = AggregatorBatchWeights::new(0, 0);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_some());

            let diff = result.unwrap();
            assert_eq!(diff.l1_gas, 1000);
            assert_eq!(diff.message_segment_length, 500);
        }

        #[test]
        fn test_checked_sub_equal_values() {
            let weights1 = AggregatorBatchWeights::new(1000, 500);
            let weights2 = AggregatorBatchWeights::new(1000, 500);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_some());

            let diff = result.unwrap();
            assert_eq!(diff.l1_gas, 0);
            assert_eq!(diff.message_segment_length, 0);
        }

        #[test]
        fn test_checked_sub_underflow_l1_gas() {
            let weights1 = AggregatorBatchWeights::new(100, 500);
            let weights2 = AggregatorBatchWeights::new(200, 300);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_none());
        }

        #[test]
        fn test_checked_sub_underflow_message_segment_length() {
            let weights1 = AggregatorBatchWeights::new(500, 100);
            let weights2 = AggregatorBatchWeights::new(300, 200);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_none());
        }

        #[test]
        fn test_checked_sub_from_max() {
            let weights1 = AggregatorBatchWeights::new(usize::MAX, usize::MAX);
            let weights2 = AggregatorBatchWeights::new(1, 1);

            let result = weights1.checked_sub(&weights2);
            assert!(result.is_some());

            let diff = result.unwrap();
            assert_eq!(diff.l1_gas, usize::MAX - 1);
            assert_eq!(diff.message_segment_length, usize::MAX - 1);
        }

        #[test]
        fn test_from_bouncer_weights() {
            let bouncer_weights =
                BouncerWeights { l1_gas: 1234, message_segment_length: usize::MAX, ..Default::default() };

            let agg_weights = AggregatorBatchWeights::from(&bouncer_weights);
            assert_eq!(agg_weights.l1_gas, 1234);
            assert_eq!(agg_weights.message_segment_length, usize::MAX);
        }
    }
}
