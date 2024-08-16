use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::WrapErr;
use prover_client_interface::{Task, TaskStatus};
use thiserror::Error;
use tracing::log::log;
use tracing::log::Level::Error;
use uuid::Uuid;

use super::constants::JOB_METADATA_CAIRO_PIE_PATH_KEY;
use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;

#[derive(Error, Debug, PartialEq)]
pub enum ProvingError {
    #[error("Cairo PIE path is not specified - prover job #{internal_id:?}")]
    CairoPIEWrongPath { internal_id: String },

    #[error("Not able to read the cairo PIE file from the zip file provided.")]
    CairoPIENotReadable,

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

pub struct ProvingJob;

#[async_trait]
impl Job for ProvingJob {
    async fn create_job(
        &self,
        _config: &Config,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        if !metadata.contains_key(JOB_METADATA_CAIRO_PIE_PATH_KEY) {
            // TODO: validate the usage of `.clone()` here, ensure lightweight borrowing of variables
            Err(ProvingError::CairoPIEWrongPath { internal_id: internal_id.clone() })?
        }
        Ok(JobItem {
            id: Uuid::new_v4(),
            internal_id,
            job_type: JobType::ProofCreation,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
        })
    }

    async fn process_job(&self, config: &Config, job: &mut JobItem) -> Result<String, JobError> {
        // TODO: allow to download PIE from storage
        let cairo_pie_path = job
            .metadata
            .get(JOB_METADATA_CAIRO_PIE_PATH_KEY)
            .map(|s| PathBuf::from_str(s))
            .ok_or_else(|| ProvingError::CairoPIEWrongPath { internal_id: job.internal_id.clone() })?
            .map_err(|_| ProvingError::CairoPIENotReadable)?;

        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).map_err(|_| ProvingError::CairoPIENotReadable)?;

        let external_id = config
            .prover_client()
            .submit_task(Task::CairoPie(cairo_pie))
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| JobError::Other(OtherError(e)))?;

        Ok(external_id)
    }

    async fn verify_job(&self, config: &Config, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
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
