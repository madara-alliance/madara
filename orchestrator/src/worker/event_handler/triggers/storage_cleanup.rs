use crate::core::client::lock::LockValue;
use crate::core::config::Config;
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::types::jobs::types::{JobStatus, JobType};
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// Worker key for distributed locking
pub const STORAGE_CLEANUP_WORKER_KEY: &str = "StorageCleanupWorker";

/// Tag key used to mark objects for expiration
const EXPIRATION_TAG_KEY: &str = "expire-after-settlement";
const EXPIRATION_TAG_VALUE: &str = "true";

/// Maximum number of jobs to process per run
const MAX_JOBS_PER_RUN: usize = 20;

/// Lock duration in seconds (5 minutes)
const CLEANUP_WORKER_LOCK_DURATION: u64 = 300;

pub struct StorageCleanupTrigger;

#[async_trait]
impl JobTrigger for StorageCleanupTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        info!("StorageCleanupTrigger started");

        // Try to acquire distributed lock
        match config
            .lock()
            .acquire_lock(STORAGE_CLEANUP_WORKER_KEY, LockValue::Boolean(false), CLEANUP_WORKER_LOCK_DURATION, None)
            .await
        {
            Ok(_) => {
                debug!("StorageCleanupTrigger acquired distributed lock");
            }
            Err(err) => {
                debug!("StorageCleanupTrigger could not acquire lock (another instance may be running): {}", err);
                return Ok(());
            }
        }

        // Execute main work and ensure lock is released
        let result = self.process_completed_jobs(&config).await;

        // Always release the lock
        if let Err(e) = config.lock().release_lock(STORAGE_CLEANUP_WORKER_KEY, None).await {
            error!("Failed to release {} lock: {}", STORAGE_CLEANUP_WORKER_KEY, e);
            if result.is_ok() {
                return Err(e.into());
            }
        } else {
            debug!("StorageCleanupTrigger released distributed lock");
        }

        info!("StorageCleanupTrigger completed");
        result
    }

    /// Storage cleanup should always run, regardless of failed jobs
    async fn is_worker_enabled(&self, _config: Arc<Config>) -> color_eyre::Result<bool> {
        Ok(true)
    }
}

impl StorageCleanupTrigger {
    /// Process completed StateTransition jobs that haven't had their artifacts tagged
    async fn process_completed_jobs(&self, config: &Arc<Config>) -> color_eyre::Result<()> {
        // Get completed StateTransition jobs
        let completed_jobs = config
            .database()
            .get_jobs_by_types_and_statuses(
                vec![JobType::StateTransition],
                vec![JobStatus::Completed],
                Some(MAX_JOBS_PER_RUN as i64),
                None, // Don't filter by orchestrator version - clean up all completed jobs
            )
            .await?;

        if completed_jobs.is_empty() {
            debug!("No completed StateTransition jobs found");
            return Ok(());
        }

        let total_completed = completed_jobs.len();

        // Filter to jobs that haven't been tagged yet
        let jobs_to_tag: Vec<_> = completed_jobs
            .into_iter()
            .filter(|job| {
                let metadata_result: Result<StateUpdateMetadata, _> = job.metadata.specific.clone().try_into();
                if let Ok(metadata) = metadata_result {
                    metadata.storage_artifacts_tagged_at.is_none()
                } else {
                    false
                }
            })
            .take(MAX_JOBS_PER_RUN)
            .collect();

        if jobs_to_tag.is_empty() {
            debug!("Found {} completed StateTransition jobs, all already have artifacts tagged", total_completed);
            return Ok(());
        }

        info!("Found {} completed StateTransition jobs, {} need artifact tagging", total_completed, jobs_to_tag.len());

        let total_to_process = jobs_to_tag.len();
        let mut jobs_processed = 0;
        let mut total_artifacts_tagged = 0;

        for job in jobs_to_tag {
            let job_id = job.internal_id;

            // Get the metadata to find artifact paths
            let state_metadata: StateUpdateMetadata = match job.metadata.specific.clone().try_into() {
                Ok(m) => m,
                Err(e) => {
                    error!(job_id = %job_id, error = %e, "Failed to parse StateUpdateMetadata");
                    continue;
                }
            };

            // Collect all artifact paths to tag
            let mut paths_to_tag = Vec::new();

            if let Some(path) = &state_metadata.snos_output_path {
                paths_to_tag.push(path.clone());
            }
            if let Some(path) = &state_metadata.program_output_path {
                paths_to_tag.push(path.clone());
            }
            if let Some(path) = &state_metadata.blob_data_path {
                paths_to_tag.push(path.clone());
            }
            if let Some(path) = &state_metadata.da_segment_path {
                paths_to_tag.push(path.clone());
            }

            if paths_to_tag.is_empty() {
                warn!(job_id = %job_id, "No artifact paths found in StateTransition job metadata");
                continue;
            }

            info!(
                job_id = %job_id,
                artifact_count = paths_to_tag.len(),
                paths = ?paths_to_tag,
                "Tagging artifacts for expiration"
            );

            // Tag all artifacts
            let tags = vec![(EXPIRATION_TAG_KEY.to_string(), EXPIRATION_TAG_VALUE.to_string())];
            let mut all_tagged = true;
            let mut tagged_count = 0;

            for path in &paths_to_tag {
                if let Err(e) = config.storage().tag_object(path, tags.clone()).await {
                    error!(job_id = %job_id, path = %path, error = %e, "Failed to tag artifact");
                    all_tagged = false;
                    break;
                }
                tagged_count += 1;
                debug!(job_id = %job_id, path = %path, "Tagged artifact for expiration");
            }

            if !all_tagged {
                // If any tagging failed, skip updating the job and try again next run
                warn!(
                    job_id = %job_id,
                    tagged = tagged_count,
                    total = paths_to_tag.len(),
                    "Partial tagging failure, will retry next run"
                );
                continue;
            }

            // Update job metadata to mark artifacts as tagged
            let mut updated_metadata = state_metadata;
            updated_metadata.storage_artifacts_tagged_at = Some(Utc::now());

            let job_update = JobItemUpdates::new().update_metadata(JobMetadata {
                common: job.metadata.common.clone(),
                specific: JobSpecificMetadata::StateUpdate(updated_metadata),
            });

            if let Err(e) = config.database().update_job(&job, job_update).await {
                error!(job_id = %job_id, error = %e, "Failed to update job metadata after tagging");
                continue;
            }

            jobs_processed += 1;
            total_artifacts_tagged += paths_to_tag.len();
            info!(job_id = %job_id, artifact_count = paths_to_tag.len(), "Successfully tagged artifacts for expiration");
        }

        info!(
            "Storage cleanup summary: processed {}/{} jobs, tagged {} artifacts total",
            jobs_processed, total_to_process, total_artifacts_tagged
        );

        Ok(())
    }
}
