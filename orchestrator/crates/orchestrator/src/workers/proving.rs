use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::KeyValue;

use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::workers::Worker;

pub struct ProvingWorker;

#[async_trait]
impl Worker for ProvingWorker {
    /// 1. Fetch all successful SNOS job runs that don't have a proving job
    /// 2. Create a proving job for each SNOS job run
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "ProvingWorker", "ProvingWorker started.");

        let successful_snos_jobs = config
            .database()
            .get_jobs_without_successor(JobType::SnosRun, JobStatus::Completed, JobType::ProofCreation)
            .await?;

        tracing::debug!("Found {} successful SNOS jobs without proving jobs", successful_snos_jobs.len());

        for job in successful_snos_jobs {
            tracing::debug!(job_id = %job.internal_id, "Creating proof creation job for SNOS job");
            match create_job(JobType::ProofCreation, job.internal_id.to_string(), job.metadata, config.clone()).await {
                Ok(_) => tracing::info!(block_id = %job.internal_id, "Successfully created new proving job"),
                Err(e) => {
                    tracing::warn!(job_id = %job.internal_id, error = %e, "Failed to create new state transition job");
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::ProofCreation)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        tracing::trace!(log_type = "completed", category = "ProvingWorker", "ProvingWorker completed.");
        Ok(())
    }
}
