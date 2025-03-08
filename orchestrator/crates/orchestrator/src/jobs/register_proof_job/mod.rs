use std::sync::Arc;

use async_trait::async_trait;
use chrono::{SubsecRound, Utc};
use color_eyre::Result;
use uuid::Uuid;

use super::JobError;
use crate::config::Config;
use crate::helpers;
use crate::jobs::metadata::JobMetadata;
use crate::jobs::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use crate::jobs::Job;

pub struct RegisterProofJob;

#[async_trait]
impl Job for RegisterProofJob {
    #[tracing::instrument(fields(category = "proof_registry"), skip(self, _config, metadata), ret, err)]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: JobMetadata,
    ) -> Result<JobItem, JobError> {
        tracing::info!(log_type = "starting", category = "proof_registry", function_type = "create_job",  block_no = %internal_id, "Proof registration job creation started.");
        let job_item = JobItem {
            id: Uuid::new_v4(),
            internal_id: internal_id.clone(),
            job_type: JobType::ProofRegistration,
            status: JobStatus::Created,
            external_id: String::new().into(),
            // metadata must contain the blocks that have been included inside this proof
            // this will allow state update jobs to be created for each block
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        };
        tracing::info!(log_type = "completed", category = "proof_registry", function_type = "create_job",  block_no = %internal_id,  "Proof registration job created.");
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "proof_registry"), skip(self, _config), ret, err)]
    async fn process_job(&self, _config: Arc<Config>, _job: &mut JobItem) -> Result<String, JobError> {
        // Get proof from storage and submit on chain for verification
        // We need to implement a generic trait for this to support multiple
        // base layers
        todo!()
    }

    #[tracing::instrument(fields(category = "proof_registry"), skip(self, _config), ret, err)]
    async fn verify_job(&self, _config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "proof_registry", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id, "Proof registration job verification started.");
        // verify that the proof transaction has been included on chain
        tracing::info!(log_type = "completed", category = "proof_registry", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id, "Proof registration job verification completed.");
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

    fn job_processing_lock(
        &self,
        _config: Arc<Config>,
    ) -> std::option::Option<std::sync::Arc<helpers::JobProcessingState>> {
        None
    }
}
