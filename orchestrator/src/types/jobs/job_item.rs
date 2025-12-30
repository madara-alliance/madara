use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::{chrono_datetime_as_bson_datetime, uuid_1_as_binary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Custom serde module for serializing u64 as i64 in BSON.
/// MongoDB's NumberLong is a signed 64-bit integer, but internal_id is always positive.
/// This module handles the conversion while keeping the Rust type as u64 for type safety.
#[cfg(feature = "with_mongodb")]
mod u64_as_bson_int64 {
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Safe to cast since block/batch numbers won't exceed i64::MAX
        serializer.serialize_i64(*value as i64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = i64::deserialize(deserializer)?;
        if value < 0 {
            return Err(serde::de::Error::custom("internal_id cannot be negative"));
        }
        Ok(value as u64)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JobItem {
    /// an uuid to identify a job
    #[cfg_attr(feature = "with_mongodb", serde(with = "uuid_1_as_binary"))]
    pub id: Uuid,
    /// a meaningful id used to track a job internally, ex: block_no, batch_no
    #[cfg_attr(feature = "with_mongodb", serde(with = "u64_as_bson_int64"))]
    pub internal_id: u64,
    /// the type of job
    pub job_type: JobType,
    /// the status of the job
    pub status: JobStatus,
    /// external id to track the status of the job. for ex, txn hash for blob inclusion
    /// or job_id from SHARP
    pub external_id: ExternalId,
    /// additional field to store values related to the job
    pub metadata: JobMetadata,
    /// helps to keep track of the version of the item for optimistic locking
    pub version: i32,
    /// timestamp when the job was created
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub created_at: DateTime<Utc>,
    /// timestamp when the job was last updated
    #[cfg_attr(feature = "with_mongodb", serde(with = "chrono_datetime_as_bson_datetime"))]
    pub updated_at: DateTime<Utc>,
}

impl JobItem {
    /// Creates a new job item with the given parameters.
    ///
    /// # Arguments
    /// * `internal_id` - A numeric ID representing the internal ID of the job (e.g., block_no, batch_no).
    /// * `job_type` - The type of the job.
    /// * `status` - The status of the job.
    /// * `metadata` - The metadata associated with the job.
    ///
    /// # Returns
    /// A new `JobItem` instance with the specified parameters.
    pub fn create(internal_id: u64, job_type: JobType, status: JobStatus, metadata: JobMetadata) -> Self {
        Self {
            id: Uuid::new_v4(),
            internal_id,
            job_type,
            status,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        }
    }
}
