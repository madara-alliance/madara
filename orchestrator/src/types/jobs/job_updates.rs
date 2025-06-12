use crate::types::jobs::external_id::ExternalId;
use crate::types::jobs::metadata::JobMetadata;
use crate::types::jobs::types::JobStatus;
use crate::error::job::JobError;
use serde::Serialize;

/// Defining a structure that contains the changes to be made in the job object,
/// id and created at are not allowed to be changed
// version and updated_at will always be updated when this object updates the job
#[derive(Serialize, Debug)]
pub struct JobItemUpdates {
    pub status: Option<JobStatus>,
    pub external_id: Option<ExternalId>,
    pub metadata: Option<JobMetadata>,
}

/// implements only needed singular changes
impl Default for JobItemUpdates {
    fn default() -> Self {
        Self::new()
    }
}

impl JobItemUpdates {
    pub fn new() -> Self {
        JobItemUpdates { status: None, external_id: None, metadata: None }
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
    pub fn build(self) -> Result<JobItemUpdates, JobError> {
        if self.status.is_none() && self.external_id.is_none() && self.metadata.is_none() {
            Err(JobError::Other("No field to be updated, likely a false call".to_string()))
        } else {
            Ok(self)
        }
    }
}
