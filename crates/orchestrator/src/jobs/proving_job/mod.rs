use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use prover_client_interface::{Task, TaskStatus};
use tracing::log::log;
use tracing::log::Level::Error;
use uuid::Uuid;

use super::constants::JOB_METADATA_CAIRO_PIE_PATH_KEY;
use super::types::{JobItem, JobStatus, JobType, JobVerificationStatus};
use super::Job;
use crate::config::Config;

pub struct ProvingJob;

#[async_trait]
impl Job for ProvingJob {
    async fn create_job(
        &self,
        _config: &Config,
        internal_id: String,
        metadata: HashMap<String, String>,
    ) -> Result<JobItem> {
        if !metadata.contains_key(JOB_METADATA_CAIRO_PIE_PATH_KEY) {
            return Err(eyre!("Cairo PIE path is not specified (prover job #{})", internal_id));
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

    async fn process_job(&self, config: &Config, job: &JobItem) -> Result<String> {
        // TODO: allow to donwload PIE from S3
        let cairo_pie_path: PathBuf = job
            .metadata
            .get(JOB_METADATA_CAIRO_PIE_PATH_KEY)
            .map(|s| PathBuf::from_str(s))
            .ok_or_else(|| eyre!("Cairo PIE path is not specified (prover job #{})", job.internal_id))??;
        let cairo_pie = CairoPie::read_zip_file(&cairo_pie_path).unwrap();
        let external_id = config.prover_client().submit_task(Task::CairoPie(cairo_pie)).await?;
        Ok(external_id)
    }

    async fn verify_job(&self, config: &Config, job: &JobItem) -> Result<JobVerificationStatus> {
        let task_id: String = job.external_id.unwrap_string()?.into();
        match config.prover_client().get_task_status(&task_id).await? {
            TaskStatus::Processing => Ok(JobVerificationStatus::Pending),
            TaskStatus::Succeeded => Ok(JobVerificationStatus::Verified),
            TaskStatus::Failed(err) => {
                log!(Error, "Prover job #{} failed: {}", job.internal_id, err);
                Ok(JobVerificationStatus::Rejected)
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
