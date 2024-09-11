use crate::config::Config;
use crate::jobs::create_job;
use crate::jobs::types::{JobStatus, JobType};
use crate::workers::Worker;
use async_trait::async_trait;
use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

pub struct DataSubmissionWorker;

#[async_trait]
impl Worker for DataSubmissionWorker {
    // 0. All ids are assumed to be block numbers.
    // 1. Fetch the latest completed Proving job.
    // 2. Fetch the latest DA job creation.
    // 3. Create jobs from after the lastest DA job already created till latest completed proving job.
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        // provides latest completed proof creation job id
        let latest_proven_job_id = config
            .database()
            .get_latest_job_by_type_and_status(JobType::ProofCreation, JobStatus::Completed)
            .await
            .unwrap()
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        // provides latest triggered data submission job id
        let latest_data_submission_job_id = config
            .database()
            .get_latest_job_by_type(JobType::DataSubmission)
            .await
            .unwrap()
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        let latest_data_submission_id: u64 = latest_data_submission_job_id.parse()?;
        let latest_proven_id: u64 = latest_proven_job_id.parse()?;

        // creating data submission jobs for latest blocks that don't have existing data submission jobs yet.
        for new_job_id in latest_data_submission_id + 1..latest_proven_id + 1 {
            create_job(JobType::DataSubmission, new_job_id.to_string(), HashMap::new(), config.clone()).await?;
        }

        Ok(())
    }
}
