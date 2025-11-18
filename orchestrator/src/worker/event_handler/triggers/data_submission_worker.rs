use crate::core::config::Config;
use crate::types::constant::BLOB_DATA_FILE_NAME;
use crate::types::jobs::metadata::{CommonMetadata, DaMetadata, JobMetadata, JobSpecificMetadata, ProvingMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::types::Layer;
use crate::utils::filter_jobs_by_orchestrator_version;
use crate::utils::metrics::ORCHESTRATOR_METRICS;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use std::sync::Arc;
use tracing::{error};

pub struct DataSubmissionJobTrigger;

#[async_trait]
impl JobTrigger for DataSubmissionJobTrigger {
    // 0. All ids are assumed to be block numbers.
    // 1. Fetch the latest completed Proving jobs without Data Submission jobs as successor jobs
    // 2. Create jobs.
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Self-healing: recover any orphaned DataSubmission jobs before creating new ones
        if let Err(e) = self.heal_orphaned_jobs(config.clone(), JobType::DataSubmission).await {
            error!(error = %e, "Failed to heal orphaned DataSubmission jobs, continuing with normal processing");
        }

        let previous_job_type = match config.layer() {
            Layer::L2 => JobType::ProofCreation,
            Layer::L3 => JobType::ProofRegistration,
        };

        let successful_proving_jobs = config
            .database()
            .get_jobs_without_successor(previous_job_type, JobStatus::Completed, JobType::DataSubmission)
            .await?;

        let successful_proving_jobs = filter_jobs_by_orchestrator_version(successful_proving_jobs);

        for proving_job in successful_proving_jobs {
            // Extract proving metadata
            let proving_metadata: ProvingMetadata = proving_job.metadata.specific.try_into().map_err(|e| {
                error!(
                    job_id = %proving_job.internal_id,
                    error = %e,
                    "Invalid metadata type for proving job"
                );
                e
            })?;

            // Create DA metadata
            let da_metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Da(DaMetadata {
                    block_number: proving_metadata.block_number,
                    // Set the blob data path using block number
                    blob_data_path: Some(format!("{}/{BLOB_DATA_FILE_NAME}", proving_job.internal_id)),
                    // These will be populated during processing
                    tx_hash: None,
                }),
            };

            match JobHandlerService::create_job(
                JobType::DataSubmission,
                proving_job.internal_id.clone(),
                da_metadata,
                config.clone(),
            )
            .await
            {
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e,"Failed to create new {:?} job for {}", JobType::DataSubmission, proving_job.internal_id);
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::DataSubmission)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}
