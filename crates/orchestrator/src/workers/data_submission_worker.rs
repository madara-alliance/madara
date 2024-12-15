use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use opentelemetry::KeyValue;

use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::metrics::ORCHESTRATOR_METRICS;
use crate::workers::Worker;

pub struct DataSubmissionWorker;

#[async_trait]
impl Worker for DataSubmissionWorker {
    // 0. All ids are assumed to be block numbers.
    // 1. Fetch the latest completed Proving jobs without Data Submission jobs as successor jobs
    // 2. Create jobs.
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        tracing::trace!(log_type = "starting", category = "DataSubmissionWorker", "DataSubmissionWorker started.");

        let successful_proving_jobs = config
            .database()
            .get_jobs_without_successor(JobType::ProofCreation, JobStatus::Completed, JobType::DataSubmission)
            .await?;

        for job in successful_proving_jobs {
            match create_job(JobType::DataSubmission, job.internal_id.clone(), HashMap::new(), config.clone()).await {
                Ok(_) => tracing::info!(block_id = %job.internal_id, "Successfully created new data submission job"),
                Err(e) => {
                    tracing::warn!(block_id = %job.internal_id, error = %e, "Failed to create new data submission job");
                    let attributes = [
                        KeyValue::new("operation_job_type", format!("{:?}", JobType::DataSubmission)),
                        KeyValue::new("operation_type", format!("{:?}", "create_job")),
                    ];
                    ORCHESTRATOR_METRICS.failed_job_operations.add(1.0, &attributes);
                }
            }
        }

        tracing::trace!(log_type = "completed", category = "DataSubmissionWorker", "DataSubmissionWorker completed.");
        Ok(())
    }
}
