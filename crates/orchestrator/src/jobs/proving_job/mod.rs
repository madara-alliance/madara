use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::{eyre, WrapErr};
use prover_client_interface::{Task, TaskStatus};
use thiserror::Error;
use uuid::Uuid;

use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;
use crate::constants::CAIRO_PIE_FILE_NAME;
use crate::jobs::constants::JOB_METADATA_SNOS_FACT;

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
    #[tracing::instrument(fields(category = "proving"), skip(self, _config, metadata), ret, err)]
    async fn create_job(
        &self,
        _config: Arc<Config>,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem, JobError> {
        tracing::info!(log_type = "starting", category = "proving", function_type = "create_job",  block_no = %internal_id, "Proving job creation started.");
        let job_item = JobItem {
            id: Uuid::new_v4(),
            internal_id: internal_id.clone(),
            job_type: JobType::ProofCreation,
            status: JobStatus::Created,
            external_id: String::new().into(),
            metadata,
            version: 0,
            created_at: Utc::now().round_subsecs(0),
            updated_at: Utc::now().round_subsecs(0),
        };
        tracing::info!(log_type = "completed", category = "proving", function_type = "create_job",  block_no = %internal_id, "Proving job created.");
        Ok(job_item)
    }

    #[tracing::instrument(fields(category = "proving"), skip(self, config), ret, err)]
    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "proving", function_type = "process_job", job_id = ?job.id,  block_no = %internal_id, "Proving job processing started.");

        // Cairo Pie path in s3 storage client
        let block_number: String = job.internal_id.to_string();
        let cairo_pie_path = block_number + "/" + CAIRO_PIE_FILE_NAME;
        tracing::debug!(job_id = %job.internal_id, %cairo_pie_path, "Fetching Cairo PIE file");

        let cairo_pie_file = config.storage().get_data(&cairo_pie_path).await.map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Failed to fetch Cairo PIE file");
            ProvingError::CairoPIEFileFetchFailed(e.to_string())
        })?;
        tracing::debug!(job_id = %job.internal_id, "Parsing Cairo PIE file");
        let cairo_pie = Box::new(CairoPie::from_bytes(cairo_pie_file.to_vec().as_slice()).map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Failed to parse Cairo PIE file");
            ProvingError::CairoPIENotReadable(e.to_string())
        })?);

        tracing::debug!(job_id = %job.internal_id, "Submitting task to prover client");
        let external_id = config
            .prover_client()
            .submit_task(Task::CairoPie(cairo_pie), *config.prover_layout_name())
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to submit task to prover client");
                JobError::Other(OtherError(e))
            })?;

        tracing::info!(log_type = "completed", category = "proving", function_type = "process_job", job_id = ?job.id,  block_no = %internal_id, %external_id, "Proving job processed successfully.");
        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "proving"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(log_type = "starting", category = "proving", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id, "Proving job verification started.");

        let task_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to unwrap external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

        let fact = job.metadata.get(JOB_METADATA_SNOS_FACT).ok_or_else(|| {
            tracing::error!(job_id = %job.internal_id, "Fact not available in job metadata");
            OtherError(eyre!("Fact not available in job"))
        })?;

        tracing::debug!(job_id = %job.internal_id, %task_id, "Getting task status from prover client");
        let task_status = config
            .prover_client()
            .get_task_status(&task_id, fact)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to get task status from prover client");
                JobError::Other(OtherError(e))
            })?;

        match task_status {
            TaskStatus::Processing => {
                tracing::info!(log_type = "pending", category = "proving", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id,   "Proving job verification pending.");
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // TODO: call isValid on the contract over here to cross-verify whether the proof was registered on
                // chain or not

                tracing::info!(log_type = "completed", category = "proving", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id,     "Proving job verification completed.");
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                tracing::info!(log_type = "failed", category = "proving", function_type = "verify_job", job_id = ?job.id,  block_no = %internal_id,     "Proving job verification failed.");
                Ok(JobVerificationStatus::Rejected(format!(
                    "Prover job #{} failed with error: {}",
                    job.internal_id, err
                )))
            }
        }
    }

    fn max_process_attempts(&self) -> u64 {
        2
    }

    fn max_verification_attempts(&self) -> u64 {
        1200
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }
}
