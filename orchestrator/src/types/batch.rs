use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::{chrono_datetime_as_bson_datetime, uuid_1_as_binary};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, strum_macros::Display, Eq, Default)]
pub enum BatchStatus {
    /// Batch is open and new blocks can be added to it
    #[default]
    Open,
    /// Batch is closed and no new blocks can be added to it
    Closed,
    /// Batch is being processed by the aggregator job
    /// This means that all the child jobs completed by the prover client, and we can close the bucket
    RunningAggregator,
    /// Batch is closed, and we are waiting for SUCCESS from the prover client for the bucket ID
    PendingVerification,
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
    pub end_block: u64,
    pub is_batch_ready: bool,
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

// The below structs and methods are used in E2E testing of batching logic

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DataJson {
    pub state_update_size: u64,
    pub state_update: Vec<ContractUpdate>,
    pub class_declaration_size: u64,
    pub class_declaration: Vec<ClassDeclaration>,
}

// Define the data structures
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ContractUpdate {
    #[serde(serialize_with = "serialize_biguint")]
    pub address: BigUint,
    pub nonce: u64,
    pub number_of_storage_updates: u64,
    #[serde(serialize_with = "serialize_option_biguint")]
    pub new_class_hash: Option<BigUint>, // Present only if class_info_flag is 1
    pub storage_updates: Vec<StorageUpdate>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct StorageUpdate {
    #[serde(serialize_with = "serialize_biguint")]
    pub key: BigUint,
    #[serde(serialize_with = "serialize_biguint")]
    pub value: BigUint,
}

#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq, Hash)]
pub struct ClassDeclaration {
    #[serde(serialize_with = "serialize_biguint")]
    pub class_hash: BigUint,
    #[serde(serialize_with = "serialize_biguint")]
    pub compiled_class_hash: BigUint,
}

// Custom serializer for BigUint
fn serialize_biguint<S>(biguint: &BigUint, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&biguint.to_str_radix(10))
}

// Custom serializer for Option<BigUint>
fn serialize_option_biguint<S>(option_biguint: &Option<BigUint>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match option_biguint {
        Some(value) => serialize_biguint(value, serializer),
        None => serializer.serialize_none(),
    }
}

// Trait for unordered equality
pub trait UnorderedEq {
    fn unordered_eq(&self, other: &Self) -> bool;
}

// Implement UnorderedEq for DataJson
impl UnorderedEq for DataJson {
    fn unordered_eq(&self, other: &Self) -> bool {
        self.state_update.unordered_eq(&other.state_update)
            && self.class_declaration.unordered_eq(&other.class_declaration)
    }
}

// Implement UnorderedEq for Vec<ContractUpdate>
impl UnorderedEq for Vec<ContractUpdate> {
    fn unordered_eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        let mut self_sorted = self.clone();
        let mut other_sorted = other.clone();

        self_sorted.sort_by_key(|update| update.address.clone());
        other_sorted.sort_by_key(|update| update.address.clone());

        for (self_update, other_update) in self_sorted.iter().zip(other_sorted.iter()) {
            if !self_update.unordered_eq(other_update) {
                return false;
            }
        }

        true
    }
}

// Implement UnorderedEq for Vec<ClassDeclaration>
impl UnorderedEq for Vec<ClassDeclaration> {
    fn unordered_eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        let set_self: HashSet<_> = self.iter().collect();
        let set_other: HashSet<_> = other.iter().collect();

        set_self == set_other
    }
}

// Implement UnorderedEq for Vec<StorageUpdate>
impl UnorderedEq for Vec<StorageUpdate> {
    fn unordered_eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }

        let mut self_sorted = self.clone();
        let mut other_sorted = other.clone();

        self_sorted.sort_by_key(|update| update.key.clone());
        other_sorted.sort_by_key(|update| update.key.clone());

        self_sorted == other_sorted
    }
}

// Implement UnorderedEq for ContractUpdate
impl UnorderedEq for ContractUpdate {
    fn unordered_eq(&self, other: &Self) -> bool {
        self.address == other.address
            && self.nonce == other.nonce
            && self.number_of_storage_updates == other.number_of_storage_updates
            && self.new_class_hash == other.new_class_hash
            && self.storage_updates.unordered_eq(&other.storage_updates)
    }
}
