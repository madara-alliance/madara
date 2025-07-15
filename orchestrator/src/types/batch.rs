use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::{chrono_datetime_as_bson_datetime, uuid_1_as_binary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, strum_macros::Display, Eq, Default)]
pub enum BatchStatus {
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

#[derive(Serialize, Debug, Clone)]
pub struct BatchUpdates {
    pub end_block: Option<u64>,
    pub is_batch_ready: Option<bool>,
    pub status: Option<BatchStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Batch {
    /// Unique identifier for the batch
    #[cfg_attr(feature = "with_mongodb", serde(rename = "_id", with = "uuid_1_as_binary"))]
    pub id: Uuid,
    /// Index of the batch
    pub index: u64,
    /// Number of blocks in the batch
    pub num_blocks: u64,
    /// Start and end block numbers of the batch (both inclusive)
    pub start_block: u64,
    pub end_block: u64,
    /// Whether the batch is ready to be processed,
    /// This will happen when adding a new block takes the size of the felt array beyond 6 * 4096
    pub is_batch_ready: bool,
    /// Path to the squashed state updates file,
    /// This is done for optimization so we don't have to create a new squashed state update from scratch
    pub squashed_state_updates_path: String,
    /// Path to the compressed state update converted to a blob
    pub blob_path: String,
    /// timestamp when the batch was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,
    /// timestamp when the batch was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
    /// Bucket ID for the batch, received from the prover client
    pub bucket_id: Option<String>,
    /// Status of the batch
    pub status: BatchStatus,
}

impl Batch {
    pub fn create(
        index: u64,
        start_block: u64,
        squashed_state_updates_path: String,
        blob_path: String,
        bucket_id: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            num_blocks: 1,
            start_block,
            end_block: start_block,
            is_batch_ready: false,
            squashed_state_updates_path,
            blob_path,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
            bucket_id,
            ..Self::default()
        }
    }
}
