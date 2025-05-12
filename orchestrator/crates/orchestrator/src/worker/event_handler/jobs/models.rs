use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::chrono_datetime_as_bson_datetime;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Debug)]
pub struct BatchUpdates {
    pub end_block: u64,
    pub is_batch_ready: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Batch {
    /// Unique identifier for the batch
    pub id: Uuid,
    /// Index of the batch
    pub index: u64,
    /// Number of blocks in the batch
    pub size: u64,
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
    /// timestamp when the job was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,
    /// timestamp when the job was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
}

impl Batch {
    pub fn create(index: u64, start_block: u64, squashed_state_updates_path: String, blob_path: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            size: 1,
            start_block,
            end_block: start_block,
            is_batch_ready: false,
            squashed_state_updates_path,
            blob_path,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        }
    }
}
