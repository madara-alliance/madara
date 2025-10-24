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

/// Update structure for Aggregator batches
///
/// Contains optional fields that can be updated for an aggregator batch.
/// Used in database update operations to specify which fields should be modified.
#[derive(Serialize, Debug, Clone)]
pub struct AggregatorBatchUpdates {
    /// Updated end SNOS batch number
    pub end_snos_batch: Option<u64>,
    /// Updated end block number
    pub end_block: Option<u64>,
    /// Updated batch-ready status
    pub is_batch_ready: Option<bool>,
    /// Updated batch status
    pub status: Option<AggregatorBatchStatus>,
}

/// Update structure for SNOS batches
///
/// Contains optional fields that can be updated for a SNOS batch.
/// Used in database update operations to specify which fields should be modified.
#[derive(Serialize, Debug, Clone)]
pub struct SnosBatchUpdates {
    /// Updated end block number
    pub end_block: Option<u64>,
    /// Updated batch status
    pub status: Option<SnosBatchStatus>,
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
    /// Unique identifier for the batch
    #[cfg_attr(feature = "with_mongodb", serde(rename = "_id", with = "uuid_1_as_binary"))]
    pub id: Uuid,

    /// Sequential index of the aggregator batch (unique, auto-incrementing)
    /// Used for ordering and referencing batches
    pub index: u64,

    /// Number of SNOS batches in the batch
    pub num_snos_batches: u64,

    /// Start SNOS batch number (inclusive)
    pub start_snos_batch: u64,

    /// End SNOS batch number (inclusive)
    pub end_snos_batch: u64,

    /// Number of blocks covered by this aggregator batch
    /// This spans across all SNOS batches contained within this aggregator batch
    pub num_blocks: u64,

    /// Start block number of the aggregator batch (inclusive)
    /// This is the lowest block number across all contained SNOS batches
    pub start_block: u64,

    /// End block number of the aggregator batch (inclusive)
    /// This is the highest block number across all contained SNOS batches
    pub end_block: u64,

    /// Whether the batch is ready to be processed
    /// This will be true when adding a new block would exceed size limits
    /// or other batching criteria are met
    pub is_batch_ready: bool,

    /// Path to the squashed state updates file
    /// This is done for optimization so we don't have to create a new squashed
    /// state update from scratch
    pub squashed_state_updates_path: String,

    /// Path to the compressed state update converted to a blob
    /// Used for data availability and storage optimization
    pub blob_path: String,

    /// Timestamp when the batch was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,

    /// Timestamp when the batch was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,

    /// Bucket ID for the batch, received from the prover client
    /// Used to track the batch in the proving system
    pub bucket_id: String,

    /// Current status of the aggregator batch
    pub status: AggregatorBatchStatus,
    /// Starknet protocol version for all blocks in this batch
    /// All blocks in a batch must have the same Starknet version for prover compatibility
    pub starknet_version: String,
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
    pub fn new(
        index: u64,
        start_snos_batch: u64,
        start_block: u64,
        squashed_state_updates_path: String,
        blob_path: String,
        bucket_id: String,
        starknet_version: String,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            num_blocks: 1,
            start_snos_batch,
            end_snos_batch: start_snos_batch,
            num_snos_batches: 1,
            start_block,
            end_block: start_block,
            is_batch_ready: false,
            squashed_state_updates_path,
            blob_path,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
            bucket_id,
            starknet_version,
            status: AggregatorBatchStatus::Open,
        }
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
#[derive(Serialize, Deserialize, Eq, PartialEq, Debug, Clone, Default)]
pub struct SnosBatch {
    /// Unique identifier for the batch
    #[cfg_attr(feature = "with_mongodb", serde(rename = "_id", with = "uuid_1_as_binary"))]
    pub id: Uuid,

    /// SNOS batch sequential ID (unique across all SNOS batches, auto-incrementing)
    /// This provides a global ordering of SNOS batches independent of aggregator batches
    pub snos_batch_id: u64,

    /// Reference to the parent aggregator batch index
    /// This establishes the hierarchical relationship between SNOS and aggregator batches
    pub aggregator_batch_index: u64,

    /// Number of blocks in this SNOS batch
    pub num_blocks: u64,

    /// Start block number of the batch (inclusive)
    pub start_block: u64,

    /// End block number of the batch (inclusive)
    pub end_block: u64,

    /// Timestamp when the batch was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,

    /// Timestamp when the batch was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,

    /// Current status of the SNOS batch
    pub status: SnosBatchStatus,
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
    /// Panics if `end_block` < `start_block`
    pub fn new(snos_batch_id: u64, aggregator_batch_index: u64, start_block: u64) -> Self {
        Self {
            id: Uuid::new_v4(),
            snos_batch_id,
            aggregator_batch_index,
            num_blocks: 1,
            start_block,
            end_block: start_block,
            status: SnosBatchStatus::Open,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        }
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
