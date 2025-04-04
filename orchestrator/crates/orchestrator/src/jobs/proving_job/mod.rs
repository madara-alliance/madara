use std::sync::Arc;

use alloy::signers::k256::pkcs8::der::oid::db::rfc4519::SN;
use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use chrono::{SubsecRound, Utc};
use color_eyre::eyre::{eyre, WrapErr};
use orchestrator_prover_client_interface::{Task, TaskStatus};
use thiserror::Error;
use uuid::Uuid;

use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::{Job, JobError, OtherError};
use crate::config::Config;
use crate::helpers;
use crate::jobs::metadata::{JobMetadata, ProvingInputType, ProvingMetadata};

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
        metadata: JobMetadata,
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
        tracing::info!(
            log_type = "starting",
            category = "proving",
            function_type = "process_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "Proving job processing started."
        );

        // Get proving metadata
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for proving job");
            JobError::Other(OtherError(e))
        })?;

        // Get input path from metadata
        let input_path = match proving_metadata.input_path {
            Some(ProvingInputType::CairoPie(path)) => path,
            Some(ProvingInputType::Proof(_)) => {
                return Err(JobError::Other(OtherError(eyre!("Expected CairoPie input, got Proof"))));
            }
            None => return Err(JobError::Other(OtherError(eyre!("Input path not found in job metadata")))),
        };

        tracing::debug!(job_id = %job.internal_id, %input_path, "Fetching Cairo PIE file");

        // Fetch and parse Cairo PIE
        let cairo_pie_file = config.storage().get_data(&input_path).await.map_err(|e| {
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

        Ok(external_id)
    }

    #[tracing::instrument(fields(category = "proving"), skip(self, config), ret, err)]
    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = job.internal_id.clone();
        tracing::info!(
            log_type = "starting",
            category = "proving",
            function_type = "verify_job",
            job_id = ?job.id,
            block_no = %internal_id,
            "Proving job verification started."
        );

        // Get proving metadata
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            tracing::error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for proving job");
            JobError::Other(OtherError(e))
        })?;

        // Get task ID from external_id
        let task_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                tracing::error!(job_id = %job.internal_id, error = %e, "Failed to unwrap external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

        // Determine if we need on-chain verification
        let (cross_verify, fact) = match &proving_metadata.ensure_on_chain_registration {
            Some(fact_str) => (true, Some(fact_str.clone())),
            None => (false, None),
        };

        tracing::debug!(
            job_id = %job.internal_id,
            %task_id,
            cross_verify,
            "Getting task status from prover client"
        );

        let task_status = config
            .prover_client()
            .get_task_status(&task_id, fact, cross_verify)
            .await
            .wrap_err("Prover Client Error".to_string())
            .map_err(|e| {
                tracing::error!(
                    job_id = %job.internal_id,
                    error = %e,
                    "Failed to get task status from prover client"
                );
                JobError::Other(OtherError(e))
            })?;

        match task_status {
            TaskStatus::Processing => {
                tracing::info!(
                    log_type = "pending",
                    category = "proving",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proving job verification pending."
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // If proof download path is specified, store the proof
                if let Some(download_path) = proving_metadata.download_proof {
                    tracing::debug!(
                        job_id = %job.internal_id,
                        "Downloading and storing proof to path: {}",
                        download_path
                    );
                    // TODO: Implement proof download and storage
                }

                tracing::info!(
                    log_type = "completed",
                    category = "proving",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proving job verification completed."
                );
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                tracing::info!(
                    log_type = "failed",
                    category = "proving",
                    function_type = "verify_job",
                    job_id = ?job.id,
                    block_no = %internal_id,
                    "Proving job verification failed."
                );
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
        300
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }

    fn job_processing_lock(&self, config: Arc<Config>) -> Option<Arc<helpers::JobProcessingState>> {
        config.processing_locks().proving_job_processing_lock.clone()
    }
}
