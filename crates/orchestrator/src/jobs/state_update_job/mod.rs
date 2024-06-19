use std::collections::HashMap;

use async_trait::async_trait;
use color_eyre::Result;
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

pub struct StateUpdateJob;

#[async_trait]
impl Job for StateUpdateJob {
    async fn create_job(
        &self,
        _config: &Config,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem> {
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id,
            job_type: JobType::ProofRegistration,
            status: JobStatus::Created,
            external_id: String::new().into(),
            // metadata must contain the blocks for which state update will be performed
            // we don't do one job per state update as that makes nonce management complicated
            metadata,
            version: 0,
        })
    }

    async fn process_job(&self, _config: &Config, _job: &JobItem) -> Result<String> {
        // Read the metadata to get the blocks for which state update will be performed.
        // For each block, get the program output (from the PIE?) and the
        todo!()
    }

    async fn verify_job(&self, _config: &Config, _job: &JobItem) -> Result<JobVerificationStatus> {
        // verify that the proof transaction has been included on chain
        todo!()
    }

    fn max_process_attempts(&self) -> u64 {
        todo!()
    }

    fn max_verification_attempts(&self) -> u64 {
        todo!()
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        todo!()
    }
}
