use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::types::{JobStatus, JobType};
use chrono::{DateTime, SubsecRound, Utc};
#[cfg(feature = "with_mongodb")]
use mongodb::bson::serde_helpers::{chrono_datetime_as_bson_datetime, uuid_1_as_binary};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct JobItem {
    /// an uuid to identify a job
    #[cfg_attr(feature = "with_mongodb", serde(with = "uuid_1_as_binary"))]
    pub id: Uuid,
    /// a meaningful id used to track a job internally, ex: block_no, txn_hash
    pub internal_id: String,
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
    /// * `internal_id` - A string representing the internal ID of the job.
    /// * `job_type` - The type of the job.
    /// * `status` - The status of the job.
    /// * `metadata` - The metadata associated with the job.
    ///
    /// # Returns
    /// A new `JobItem` instance with the specified parameters.
    pub fn create(internal_id: String, job_type: JobType, status: JobStatus, metadata: JobMetadata) -> Self {
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
