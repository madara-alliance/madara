use async_trait::async_trait;
use cairo_vm::vm::runners::cairo_pie::CairoPie;
use color_eyre::eyre::eyre;
use orchestrator_prover_client_interface::{CreateJobInfo, Task, TaskStatus, TaskType};
use std::sync::Arc;
use std::time::Instant;

use crate::core::config::Config;
use crate::error::job::proving::ProvingError;
use crate::error::job::JobError;
use crate::error::other::OtherError;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::metadata::{JobMetadata, ProvingInputType, ProvingMetadata};
use crate::types::jobs::status::JobVerificationStatus;
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::jobs::JobHandlerTrait;
use tracing::{debug, error, info, warn};

pub struct ProvingJobHandler;

#[async_trait]
impl JobHandlerTrait for ProvingJobHandler {
    async fn create_job(&self, internal_id: String, metadata: JobMetadata) -> Result<JobItem, JobError> {
        debug!(log_type = "starting", "{:?} job {} creation started", JobType::ProofCreation, internal_id);
        let job_item = JobItem::create(internal_id.clone(), JobType::ProofCreation, JobStatus::Created, metadata);
        debug!(log_type = "completed", "{:?} job {} creation completed", JobType::ProofCreation, internal_id);
        Ok(job_item)
    }

    async fn process_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<String, JobError> {
        let internal_id = &job.internal_id;
        info!(log_type = "starting", job_id = %job.id, "⚙️  {:?} job {} processing started", JobType::ProofCreation, internal_id);

        // Get proving metadata
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into().inspect_err(|e| {
            error!(error = %e, "Failed to convert metadata to ProvingMetadata");
        })?;

        // Get the input path from metadata
        let input_path = match proving_metadata.input_path {
            Some(ProvingInputType::CairoPie(path)) => path,
            Some(ProvingInputType::Proof(_)) => {
                return Err(JobError::Other(OtherError(eyre!("Expected CairoPie input, got Proof"))));
            }
            None => return Err(JobError::Other(OtherError(eyre!("Input path not found in job metadata")))),
        };

        debug!(%input_path, "Fetching Cairo PIE file");

        // Fetch and parse Cairo PIE
        let cairo_pie_file = config.storage().get_data(&input_path).await.map_err(|e| {
            error!(error = %e, "Failed to fetch Cairo PIE file");
            ProvingError::CairoPIEFileFetchFailed(e.to_string())
        })?;

        debug!("Parsing Cairo PIE file");
        let cairo_pie = Box::new(CairoPie::from_bytes(cairo_pie_file.to_vec().as_slice()).map_err(|e| {
            error!(error = %e, "Failed to parse Cairo PIE file");
            ProvingError::CairoPIENotReadable(e.to_string())
        })?);

        debug!("Submitting task to prover client");

        let proof_start = Instant::now();

        let external_id = config
            .prover_client()
            .submit_task(Task::CreateJob(CreateJobInfo {
                cairo_pie,
                bucket_id: proving_metadata.bucket_id,
                bucket_job_index: proving_metadata.bucket_job_index,
                num_steps: proving_metadata.n_steps,
                external_id: job.id.to_string(),
            }))
            .await
            .inspect_err(|e| {
                error!(error = %e, "Failed to submit task to prover client");
            })?;

        // Record proof submission time (this is just the submission, actual proof generation is async)
        let proof_duration = proof_start.elapsed().as_secs_f64();
        MetricsRecorder::record_proof_generation_time("proof_submission", proof_duration);

        info!(log_type = "completed", job_id = %job.id, "✅ {:?} job {} processed successfully", JobType::ProofCreation, internal_id);

        Ok(external_id)
    }

    async fn verify_job(&self, config: Arc<Config>, job: &mut JobItem) -> Result<JobVerificationStatus, JobError> {
        let internal_id = &job.internal_id;
        debug!(log_type = "starting", job_id = %job.id, "{:?} job {} verification started", JobType::ProofCreation, internal_id);

        // Get proving metadata
        let proving_metadata: ProvingMetadata = job.metadata.specific.clone().try_into()?;

        // Get task ID from external_id
        let task_id: String = job
            .external_id
            .unwrap_string()
            .map_err(|e| {
                error!(job_id = %job.internal_id, error = %e, "Failed to unwrap external_id");
                JobError::Other(OtherError(e))
            })?
            .into();

        // Determine if we need on-chain verification
        let (cross_verify, fact) = match &proving_metadata.ensure_on_chain_registration {
            Some(fact_str) => (true, Some(fact_str.clone())),
            None => (false, None),
        };

        debug!(
            %task_id,
            cross_verify,
            "Getting task status from prover client"
        );

        let task_status =
            config.prover_client().get_task_status(TaskType::Job, &task_id, fact, cross_verify).await.inspect_err(
                |e| {
                    error!(
                        job_id = %job.internal_id,
                        error = %e,
                        "Failed to get task status from prover client"
                    );
                },
            )?;

        match task_status {
            TaskStatus::Processing => {
                info!(
                    job_id = %job.id,
                    "{:?} job {} verification is pending, will be retried in sometime",
                    JobType::ProofCreation,
                    internal_id
                );
                Ok(JobVerificationStatus::Pending)
            }
            TaskStatus::Succeeded => {
                // If the proof download path is specified, store the proof
                if let Some(download_path) = &proving_metadata.download_proof {
                    let fetched_proof = config.prover_client().get_proof(&task_id).await.inspect_err(|e| {
                        error!(
                            error = %e,
                            "Failed to get proof from prover client"
                        );
                    })?;
                    debug!("Downloading and storing proof to path: {}", download_path);
                    config.storage().put_data(bytes::Bytes::from(fetched_proof.into_bytes()), download_path).await?;
                }
                info!(log_type = "completed", job_id = %job.id, "🎯 {:?} job {} verification completed", JobType::ProofCreation, internal_id);
                Ok(JobVerificationStatus::Verified)
            }
            TaskStatus::Failed(err) => {
                warn!(log_type = "rejected", job_id = %job.id, "❌ {:?} job {} verification failed", JobType::ProofCreation, internal_id);
                Ok(JobVerificationStatus::Rejected(format!(
                    "Prover job #{} failed with error: {}",
                    job.internal_id, err
                )))
            }
        }
    }

    fn max_process_attempts(&self) -> u64 {
        // we want this as 1 since we don't want to retry proof creation
        // because once proof creation fails, the AR bucket will also fail
        // hence, no point of retrying right now
        1
    }

    fn max_verification_attempts(&self) -> u64 {
        300
    }

    fn verification_polling_delay_seconds(&self) -> u64 {
        30
    }
}
