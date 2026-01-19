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
        // Get completed StateTransition jobs that haven't been tagged yet
        // This query filters at the database level for efficiency
        let jobs_to_tag = config
            .database()
            .get_jobs_without_storage_artifacts_tagged(Some(STORAGE_CLEANUP_MAX_JOBS_PER_RUN as i64))
            .await?;

        if jobs_to_tag.is_empty() {
            debug!("No completed StateTransition jobs need artifact tagging");
            return Ok(());
        }

        info!("Found {} completed StateTransition jobs that need artifact tagging", jobs_to_tag.len());

        let total_to_process = jobs_to_tag.len();
        let mut jobs_processed = 0;
        let mut total_artifacts_tagged = 0;

        for job in jobs_to_tag {
            let job_id = job.internal_id;

            // Get the metadata for updating later
            let state_metadata: StateUpdateMetadata = match job.metadata.specific.clone().try_into() {
                Ok(m) => m,
                Err(e) => {
                    error!(job_id = %job_id, error = %e, "Failed to parse StateUpdateMetadata");
                    continue;
                }
            };

            // List all objects in the batch folders based on internal_id
            // This is more robust than relying on metadata paths
            //
            // Storage paths to clean up:
            // 1. artifacts/batch/{job_id}/ - Aggregator artifacts (new format)
            // 2. blob/batch/{job_id}/ - Blob data files
            // 3. {job_id}/ - SNOS artifacts (old format, at root level)
            // 4. state_update/batch/{job_id}.json - State update file
            let artifact_dir = get_batch_artifacts_dir(job_id);
            let blob_dir = get_batch_blob_dir(job_id);
            let snos_dir = get_snos_legacy_dir(job_id);
            let state_update_file = get_batch_state_update_file(job_id);

            let mut paths_to_tag = Vec::new();

            // Get all files in artifacts/batch/{job_id}/ (new format)
            match config.storage().list_files_in_dir(&artifact_dir).await {
                Ok(files) => {
                    debug!(job_id = %job_id, dir = %artifact_dir, file_count = files.len(), "Found artifact files");
                    paths_to_tag.extend(files);
                }
                Err(e) => {
                    debug!(job_id = %job_id, dir = %artifact_dir, error = %e, "No artifacts directory or error listing");
                }
            }

            // Get all files in blob/batch/{job_id}/
            match config.storage().list_files_in_dir(&blob_dir).await {
                Ok(files) => {
                    debug!(job_id = %job_id, dir = %blob_dir, file_count = files.len(), "Found blob files");
                    paths_to_tag.extend(files);
                }
                Err(e) => {
                    debug!(job_id = %job_id, dir = %blob_dir, error = %e, "No blob directory or error listing");
                }
            }

            // Get all files in {job_id}/ (old SNOS format at root level)
            match config.storage().list_files_in_dir(&snos_dir).await {
                Ok(files) => {
                    debug!(job_id = %job_id, dir = %snos_dir, file_count = files.len(), "Found SNOS files (old format)");
                    paths_to_tag.extend(files);
                }
                Err(e) => {
                    debug!(job_id = %job_id, dir = %snos_dir, error = %e, "No SNOS directory or error listing");
                }
            }

            // Add state_update/batch/{job_id}.json file directly (it's a file, not a directory)
            paths_to_tag.push(state_update_file.clone());
            debug!(job_id = %job_id, file = %state_update_file, "Added state update file to tag list");

            if paths_to_tag.is_empty() {
                // No artifacts found - this is OK, mark as tagged anyway
                info!(job_id = %job_id, "No artifacts found in storage, marking job as tagged");
            } else {
                info!(
                    job_id = %job_id,
                    artifact_count = paths_to_tag.len(),
                    paths = ?paths_to_tag,
                    "Tagging artifacts for expiration"
                );
            }

            // Tag all artifacts
            let tags = vec![(STORAGE_EXPIRATION_TAG_KEY.to_string(), STORAGE_EXPIRATION_TAG_VALUE.to_string())];
            let mut all_tagged = true;
            let mut tagged_count = 0;

            for path in &paths_to_tag {
                match config.storage().tag_object(path, tags.clone()).await {
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
                            all_tagged = false;
                            break;
                        }
                    }
                }
            }

            if !all_tagged {
                // If any tagging failed (excluding not found), skip updating the job and try again next run
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
            total_artifacts_tagged += tagged_count;
            info!(job_id = %job_id, artifact_count = tagged_count, "Successfully tagged artifacts for expiration");
        }

        info!(
            "Storage cleanup summary: processed {}/{} jobs, tagged {} artifacts total",
            jobs_processed, total_to_process, total_artifacts_tagged
        );

        Ok(())
    }
}
