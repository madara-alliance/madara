use crate::core::config::Config;
use crate::types::{
    constant::ORCHESTRATOR_VERSION,
    jobs::{
        metadata::{
            AggregatorMetadata, BlobSettlementMode, CommonMetadata, JobMetadata, JobSpecificMetadata,
            SignatureCollectionMetadata,
        },
        types::{JobStatus, JobType},
    },
};
use crate::worker::event_handler::{service::JobHandlerService, triggers::JobTrigger};
use async_trait::async_trait;
use std::sync::Arc;

pub struct SignatureCollectionJobTrigger;

#[async_trait]
impl JobTrigger for SignatureCollectionJobTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        let Some(attestation) = &config.params.blob_attestation else { return Ok(()) };
        let jobs = config
            .database()
            .get_jobs_without_successor(
                JobType::Aggregator,
                JobStatus::Completed,
                JobType::SignatureCollection,
                Some(ORCHESTRATOR_VERSION.to_owned()),
                0,
            )
            .await?;
        for job in jobs {
            let aggregator: AggregatorMetadata = job.metadata.specific.try_into()?;
            if aggregator.blob_settlement_mode != BlobSettlementMode::CommitteeAttestation {
                continue;
            }
            let metadata = JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::SignatureCollection(SignatureCollectionMetadata {
                    aggregator,
                    policy: attestation.policy.clone(),
                    program_output: Vec::new(),
                    digest: None,
                    certificate: None,
                }),
            };
            JobHandlerService::create_job(JobType::SignatureCollection, job.internal_id, metadata, config.clone())
                .await?;
        }
        Ok(())
    }
}
