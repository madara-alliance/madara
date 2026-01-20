use crate::core::client::lock::LockValue;
use crate::core::config::Config;
use crate::types::constant::{
    get_batch_artifacts_dir, get_batch_blob_dir, get_batch_state_update_file, get_snos_legacy_dir,
    STORAGE_CLEANUP_LOCK_DURATION, STORAGE_CLEANUP_MAX_JOBS_PER_RUN, STORAGE_CLEANUP_WORKER_KEY,
    STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE,
};
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, StateUpdateMetadata};
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct StorageCleanupTrigger;

#[async_trait]
impl JobTrigger for StorageCleanupTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        info!("StorageCleanupTrigger started");

        // Try to acquire distributed lock
        match config
            .lock()
            .acquire_lock(STORAGE_CLEANUP_WORKER_KEY, LockValue::Boolean(false), STORAGE_CLEANUP_LOCK_DURATION, None)
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
        let jobs_to_tag = config
            .database()
            .get_jobs_without_storage_artifacts_tagged(Some(STORAGE_CLEANUP_MAX_JOBS_PER_RUN as i64))
            .await?;

        if jobs_to_tag.is_empty() {
            debug!("No completed StateTransition jobs need artifact tagging");
            return Ok(());
        }

        debug!("Found {} completed StateTransition jobs that need artifact tagging", jobs_to_tag.len());

        let total_to_process = jobs_to_tag.len();
        let mut jobs_processed = 0;
        let mut total_artifacts_tagged = 0;

        for job in jobs_to_tag {
            let job_id = job.internal_id;

            let state_metadata: StateUpdateMetadata = match job.metadata.specific.clone().try_into() {
                Ok(m) => m,
                Err(e) => {
                    error!(job_id = %job_id, error = %e, "Failed to parse StateUpdateMetadata");
                    continue;
                }
            };

            let paths_to_tag = self.collect_artifact_paths(config, job_id).await;

            if paths_to_tag.is_empty() {
                debug!(job_id = %job_id, "No artifacts found in storage, marking job as tagged");
            } else {
                debug!(
                    job_id = %job_id,
                    artifact_count = paths_to_tag.len(),
                    paths = ?paths_to_tag,
                    "Tagging artifacts for expiration"
                );
            }

            let (tagged_count, all_tagged) = self.tag_artifacts(config, job_id, &paths_to_tag).await;

            if !all_tagged {
                warn!(
                    job_id = %job_id,
                    tagged = tagged_count,
                    total = paths_to_tag.len(),
                    "Partial tagging failure, will retry next run"
                );
                continue;
            }

            if let Err(e) = self.mark_job_tagged(config, &job, state_metadata).await {
                error!(job_id = %job_id, error = %e, "Failed to update job metadata after tagging");
                continue;
            }

            jobs_processed += 1;
            total_artifacts_tagged += tagged_count;
            debug!(job_id = %job_id, artifact_count = tagged_count, "Successfully tagged artifacts for expiration");
        }

        info!(
            "Storage cleanup summary: processed {}/{} jobs, tagged {} artifacts total",
            jobs_processed, total_to_process, total_artifacts_tagged
        );

        Ok(())
    }

    /// Collect artifact paths from: artifacts/, blob/, snos legacy dirs + state_update file
    async fn collect_artifact_paths(&self, config: &Arc<Config>, job_id: u64) -> Vec<String> {
        let mut paths = Vec::new();

        // List files from each artifact directory
        let dirs = [get_batch_artifacts_dir(job_id), get_batch_blob_dir(job_id), get_snos_legacy_dir(job_id)];
        for dir in &dirs {
            match config.storage().list_files_in_dir(dir).await {
                Ok(files) => {
                    debug!(job_id = %job_id, dir = %dir, count = files.len(), "Found files");
                    paths.extend(files);
                }
                Err(e) => debug!(job_id = %job_id, dir = %dir, error = %e, "Dir not found or error"),
            }
        }

        // Add state update file (single file, always included)
        paths.push(get_batch_state_update_file(job_id));
        paths
    }

    /// Tag artifacts for expiration. Returns (tagged_count, all_succeeded).
    /// Handles non-existent files gracefully (counts as success).
    async fn tag_artifacts(&self, config: &Arc<Config>, job_id: u64, paths: &[String]) -> (usize, bool) {
        // Create tags once outside the loop - only a reference is passed, avoiding Vec clones
        let tags = vec![(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];
        let mut tagged_count = 0;

        for path in paths {
            match config.storage().tag_object(path, &tags).await {
                Ok(_) => {
                    tagged_count += 1;
                    debug!(job_id = %job_id, path = %path, "Tagged artifact for expiration");
                }
                Err(e) => {
                    let error_str = e.to_string();
                    // Handle non-existent files gracefully (file may have been deleted)
                    if error_str.contains("NoSuchKey")
                        || error_str.contains("not found")
                        || error_str.contains("does not exist")
                    {
                        debug!(job_id = %job_id, path = %path, "Artifact not found, skipping");
                        tagged_count += 1; // Count as success since there's nothing to tag
                    } else {
                        error!(job_id = %job_id, path = %path, error = %e, "Failed to tag artifact");
                        return (tagged_count, false);
                    }
                }
            }
        }

        (tagged_count, true)
    }

    /// Update job metadata to mark artifacts as tagged with current timestamp
    async fn mark_job_tagged(
        &self,
        config: &Arc<Config>,
        job: &crate::types::jobs::job_item::JobItem,
        mut state_metadata: StateUpdateMetadata,
    ) -> color_eyre::Result<()> {
        state_metadata.storage_artifacts_tagged_at = Some(Utc::now());

        let job_update = JobItemUpdates::new().update_metadata(JobMetadata {
            common: job.metadata.common.clone(),
            specific: JobSpecificMetadata::StateUpdate(state_metadata),
        });

        config.database().update_job(job, job_update).await?;
        Ok(())
    }
}
