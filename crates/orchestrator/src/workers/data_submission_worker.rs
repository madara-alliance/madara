use std::collections::HashMap;
use std::error::Error;
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
    // 1. Fetch the latest completed Proving job.
    // 2. Fetch the latest DA job creation.
    // 3. Create jobs from after the lastest DA job already created till latest completed proving job.
    async fn run_worker(&self, config: Arc<Config>) -> Result<(), Box<dyn Error>> {
        tracing::trace!(log_type = "starting", category = "DataSubmissionWorker", "DataSubmissionWorker started.");

        // provides latest completed proof creation job id
        let latest_proven_job_id = config
            .database()
            .get_latest_job_by_type_and_status(JobType::ProofCreation, JobStatus::Completed)
            .await?
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        tracing::debug!(latest_proven_job_id, "Fetched latest completed ProofCreation job");

        // provides latest triggered data submission job id
        let latest_data_submission_job_id = config
            .database()
            .get_latest_job_by_type(JobType::DataSubmission)
            .await?
            .map(|item| item.internal_id)
            .unwrap_or("0".to_string());

        tracing::debug!(latest_data_submission_job_id, "Fetched latest DataSubmission job");

        let latest_data_submission_id: u64 = match latest_data_submission_job_id.parse() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to parse latest_data_submission_job_id");
                return Err(Box::new(e));
            }
        };

        let latest_proven_id: u64 = match latest_proven_job_id.parse() {
            Ok(id) => id,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to parse latest_proven_job_id");
                return Err(Box::new(e));
            }
        };

        tracing::debug!(latest_data_submission_id, latest_proven_id, "Parsed job IDs");

        // creating data submission jobs for latest blocks that don't have existing data submission jobs
        // yet.
        for new_job_id in latest_data_submission_id + 1..latest_proven_id + 1 {
            tracing::debug!(new_job_id, "Creating new DataSubmission job");
            match create_job(JobType::DataSubmission, new_job_id.to_string(), HashMap::new(), config.clone()).await {
                Ok(_) => tracing::info!(job_id = new_job_id, "Successfully created DataSubmission job"),
                Err(e) => {
                    tracing::error!(job_id = new_job_id, error = ?e, "Failed to create DataSubmission job");
                    return Err(Box::new(e));
                }
            }
        }

        tracing::trace!(log_type = "completed", category = "DataSubmissionWorker", "DataSubmissionWorker completed.");
        Ok(())
    }
}
