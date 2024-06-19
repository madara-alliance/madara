use std::collections::HashMap;

use async_trait::async_trait;
use color_eyre::Result;
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

pub struct RegisterProofJob;

#[async_trait]
impl Job for RegisterProofJob {
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
            // metadata must contain the blocks that have been included inside this proof
            // this will allow state update jobs to be created for each block
            metadata,
            version: 0,
        })
    }

    async fn process_job(&self, _config: &Config, _job: &JobItem) -> Result<String> {
        // Get proof from S3 and submit on chain for verification
        // We need to implement a generic trait for this to support multiple
        // base layers
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
