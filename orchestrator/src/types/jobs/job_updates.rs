use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::types::JobStatus;
use chrono::{DateTime, Utc};
use serde::Serialize;

/// Defining a structure that contains the changes to be made in the job object,
/// id and created at are not allowed to be changed
// version and updated_at will always be updated when this object updates the job
#[derive(Serialize, Debug)]
pub struct JobItemUpdates {
    pub status: Option<JobStatus>,
    pub external_id: Option<ExternalId>,
    pub metadata: Option<JobMetadata>,

    // NEW FIELDS FOR GREEDY MODE
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_at: Option<Option<DateTime<Utc>>>, // Option<Option<>> to allow setting to null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<Option<String>>,
}

/// implements only needed singular changes
impl Default for JobItemUpdates {
    fn default() -> Self {
        Self::new()
    }
}

impl JobItemUpdates {
    pub fn new() -> Self {
        JobItemUpdates { status: None, external_id: None, metadata: None, available_at: None, claimed_by: None }
    }

    pub fn update_status(mut self, status: JobStatus) -> JobItemUpdates {
        self.status = Some(status);
        self
    }
    pub fn update_external_id(mut self, external_id: ExternalId) -> JobItemUpdates {
        self.external_id = Some(external_id);
        self
    }
    pub fn update_metadata(mut self, metadata: JobMetadata) -> JobItemUpdates {
        self.metadata = Some(metadata);
        self
    }

    /// Set when job becomes available for pickup
    pub fn update_available_at(mut self, available_at: Option<DateTime<Utc>>) -> Self {
        self.available_at = Some(available_at);
        self
    }

    /// Set which orchestrator claimed this job
    pub fn update_claimed_by(mut self, claimed_by: Option<String>) -> Self {
        self.claimed_by = Some(claimed_by);
        self
    }

    /// Clear the claim (release job)
    pub fn clear_claim(mut self) -> Self {
        self.claimed_by = Some(None);
        self
    }

    // creating another type JobItemUpdatesBuilder would be an overkill
    pub fn build(self) -> JobItemUpdates {
        self
    }
}
