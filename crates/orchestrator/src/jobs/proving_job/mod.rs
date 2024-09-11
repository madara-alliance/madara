use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::WrapErr;
use prover_client_interface::{Task, TaskStatus};
use thiserror::Error;
use tracing::log::log;
use tracing::log::Level::Error;
use uuid::Uuid;

use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;

#[derive(Error, Debug, PartialEq)]
pub enum ProvingError {
    #[error("Cairo PIE path is not specified - prover job #{internal_id:?}")]
    CairoPIEWrongPath { internal_id: String },

    #[error("Not able to read the cairo PIE file from the zip file provided.")]
    CairoPIENotReadable(String),

    #[error("Not able to get the PIE file from AWS S3 bucket.")]
    CairoPIEFileFetchFailed(String),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct ProvingJob;

#[async_trait]
impl Job for ProvingJob {
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id,
            job_type: JobType::ProofCreation,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        })
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        // Cairo Pie path in s3 storage client
        let cairo_pie_path = job.internal_id.to_string() + "/pie.zip";
        let cairo_pie_file = config
            .storage()
            .get_data(&cairo_pie_path)
            .await
            .map_err(|e| ProvingError::CairoPIEFileFetchFailed(e.to_string()))?;
        let cairo_pie = CairoPie::from_bytes(cairo_pie_file.to_vec().as_slice())
            .map_err(|e| ProvingError::CairoPIENotReadable(e.to_string()))?;

        let external_id = config
            .prover_client()
            .submit_task(Task::CairoPie(cairo_pie))
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        Ok(external_id)
    }

    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let task_id: String = job.external_id.unwrap_string().map_err(|e| JobError::Other(OtherError(e)))?.into();
        let task_status = config
            .prover_client()
            .get_task_status(&task_id)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        match task_status {
            TaskStatus::Processing => Ok(JobVerificationStatus::Pending),
            TaskStatus::Succeeded => Ok(JobVerificationStatus::Verified),
            TaskStatus::Failed(err) => {
                log!(Error, "Prover job #{} failed: {}", job.internal_id, err);
                Ok(JobVerificationStatus::Rejected(format!(
                    "Prover job #{} failed with error: {}",
                    job.internal_id, err
                )))
            }
        }
    }

    fn max_process_attempts(&self) -> u64 {
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        1
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        60
    }
}
