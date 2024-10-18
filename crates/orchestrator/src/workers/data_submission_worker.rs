use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
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
            create_job(JobType::DataSubmission, job.internal_id, HashMap::new(), config.clone()).await?;
        }

        tracing::trace!(log_type = "completed", category = "DataSubmissionWorker", "DataSubmissionWorker completed.");
        Ok(())
    }
}
