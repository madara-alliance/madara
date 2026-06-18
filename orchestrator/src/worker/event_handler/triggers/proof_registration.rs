use crate::core::config::Config;
use crate::types::constant::{ORCHESTRATOR_VERSION, PROOF_FILE_NAME};
use crate::types::jobs::metadata::{
    CommonMetadata, JobMetadata, JobSpecificMetadata, ProvingInputType, ProvingMetadata, SnosMetadata,
};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use opentelemetry::KeyValue;
use orchestrator_utils::layer::Layer;
use std::sync::Arc;
use tracing::{debug, error};

pub struct ProofRegistrationJobTrigger;

#[async_trait]
impl JobTrigger for ProofRegistrationJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        if matches!(config.layer(), Layer::L3) {
            return create_l3_proof_registration_jobs(config).await;
        }

        let db = config.database();

        let successful_proving_jobs = db
            .get_jobs_without_successor(
                JobType::ProofCreation,
                JobStatus::Completed,
                JobType::ProofRegistration,
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;

        debug!("Found {} successful proving jobs without proof registration jobs", successful_proving_jobs.len());

        for job in successful_proving_jobs {
            // Extract proving metadata to get relevant information
            let mut metadata = job.metadata.clone();
            let mut proving_metadata: ProvingMetadata = metadata.specific.clone().try_into().map_err(|e| {
                error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for proving job");
                e
            })?;

            // Update the input path to use proof from ProofCreation
            let proof_path = format!("{}/{}", job.internal_id, PROOF_FILE_NAME);
            proving_metadata.input_path = Some(ProvingInputType::Proof(proof_path));

            proving_metadata.download_proof = None;

            metadata.specific = JobSpecificMetadata::Proving(proving_metadata);

            debug!(job_id = %job.internal_id, "Creating proof registration job for proving job");
            match JobHandlerService::create_job(JobType::ProofRegistration, job.internal_id, metadata, config.clone())
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    error!(error = %e, "Failed to create new {:?} job for {}", JobType::ProofRegistration, job.internal_id);
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofRegistration)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    MetricsRecorder::record_failed_job_operation(1.0, &attributes);
                    return Err(e.into());
                }
            }
        }

        Ok(())
    }
}

async fn create_l3_proof_registration_jobs(config: Arc<Config>) -> color_eyre::Result<()> {
    let db = config.database();
    let successful_snos_jobs = db
        .get_jobs_without_successor(
            JobType::SnosRun,
            JobStatus::Completed,
            JobType::ProofRegistration,
            Some(ORCHESTRATOR_VERSION.to_string()),
        )
        .await?;

    debug!("Found {} successful SNOS jobs without proof registration jobs", successful_snos_jobs.len());

    for job in successful_snos_jobs {
        let snos_metadata: SnosMetadata = job.metadata.specific.clone().try_into().map_err(|e| {
            error!(job_id = %job.internal_id, error = %e, "Invalid metadata type for SNOS job");
            e
        })?;

        let metadata = JobMetadata {
            common: CommonMetadata::default(),
            specific: JobSpecificMetadata::Proving(ProvingMetadata {
                block_number: snos_metadata.start_block,
                input_path: snos_metadata.cairo_pie_path.map(ProvingInputType::CairoPie),
                ensure_on_chain_registration: None,
                n_steps: snos_metadata.snos_n_steps,
                bucket_id: None,
                bucket_job_index: None,
                download_proof: None,
            }),
        };

        debug!(job_id = %job.internal_id, "Creating proof registration job for SNOS job");
        match JobHandlerService::create_job(JobType::ProofRegistration, job.internal_id, metadata, config.clone()).await
        {
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "Failed to create new {:?} job for {}", JobType::ProofRegistration, job.internal_id);
                let attributes = [
                    KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofRegistration)),
                    KeyValue::new("operation_type", format!("{:?}", "create_job")),
                ];
                MetricsRecorder::record_failed_job_operation(1.0, &attributes);
                return Err(e.into());
            }
        }
    }

    Ok(())
}
