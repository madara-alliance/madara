use crate::core::config::Config;
use crate::types::constant::{PROOF_FILE_NAME, PROOF_PART2_FILE_NAME};
use crate::types::jobs::metadata::{JobSpecificMetadata, ProvingInputType, ProvingMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{debug, error, info, trace, warn};

pub struct ProofRegistrationJobTrigger;

#[async_trait]
impl JobTrigger for ProofRegistrationJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        trace!(log_type = "starting", category = "ProofRegistrationWorker", "ProofRegistrationWorker started.");

        // Self-healing: recover any orphaned ProofRegistration jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::ProofRegistration).await {
            error!(error = %e, "Failed to heal orphaned ProofRegistration jobs, continuing with normal processing");
        }

        let db = config.database();

        let successful_proving_jobs = db
            .get_jobs_without_successor(JobType::ProofCreation, JobStatus::Completed, JobType::ProofRegistration)
            .await?;

        info!("Found {} successful proving jobs without proof registration jobs", successful_proving_jobs.len());

        for job in successful_proving_jobs {
            // Extract proving metadata to get relevant information
            let mut metadata = job.metadata.clone();
            let mut proving_metadata: ProvingMetadata = metadata.specific.clone().try_into().map_err(|e| {
                error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for proving job");
                e
            })?;

            // Update input path to use proof from ProofCreation
            let proof_path = format!("{}/{}", job.internal_id, PROOF_FILE_NAME);
            proving_metadata.input_path = Some(ProvingInputType::Proof(proof_path));

            // Set download path for bridge proof based on layer
            proving_metadata.download_proof = match config.layer() {
                Layer::L2 => None,
                Layer::L3 => Some(format!("{}/{}", job.internal_id, PROOF_PART2_FILE_NAME)),
            };

            metadata.specific = JobSpecificMetadata::Proving(proving_metadata);

            debug!(job_id = %job.internal_id, "Creating proof registration job for proving job");
            match JobHandlerService::create_job(
                JobType::ProofRegistration,
                job.internal_id.to_string(),
                metadata,
                config.clone(),
            )
            .await
            {
                Ok(_) => info!(block_id = %job.internal_id, "Successfully created new proof registration job"),
                Err(e) => {
                    warn!(job_id = %job.internal_id, error = %e, "Failed to create new proof registration job");
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofRegistration)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        trace!(log_type = "completed", category = "ProofRegistrationWorker", "ProofRegistrationWorker completed.");
        Ok(())
    }
}
