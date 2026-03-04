use crate::core::client::lock::LockValue;
use crate::core::client::storage::StorageError;
use crate::core::config::Config;
use crate::types::constant::{
    get_batch_artifact_file, get_batch_blob_file, get_batch_state_update_file, get_snos_batch_dir, BLOB_DATA_FILE_NAME,
    CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME, MAX_BLOBS, PROGRAM_OUTPUT_FILE_NAME, PROOF_FILE_NAME,
    PROOF_PART2_FILE_NAME, SNOS_OUTPUT_FILE_NAME, STORAGE_CLEANUP_LOCK_DURATION,
    STORAGE_CLEANUP_MAX_JOBS_PER_RUN, STORAGE_CLEANUP_WORKER_KEY, STORAGE_EXPIRATION_TAG_KEY,
    STORAGE_EXPIRATION_TAG_VALUE,
};
use crate::types::jobs::job_updates::JobItemUpdates;
use crate::types::jobs::metadata::{JobMetadata, JobSpecificMetadata, SettlementContext, StateUpdateMetadata};
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::triggers::JobTrigger;
use async_trait::async_trait;
use chrono::Utc;
use std::collections::BTreeSet;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct StorageCleanupTrigger;

#[async_trait]
impl JobTrigger for StorageCleanupTrigger {
    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        // Try to acquire distributed lock
        match config
            .lock()
            .acquire_lock(STORAGE_CLEANUP_WORKER_KEY, LockValue::Boolean(false), STORAGE_CLEANUP_LOCK_DURATION, None)
            .await
        {
            Ok(_) => {
                debug!("StorageCleanupTrigger acquired distributed lock");
                info!("StorageCleanupTrigger started");
            }
            Err(err) => {
                debug!("StorageCleanupTrigger could not acquire lock (another instance may be running): {}", err);
                return Ok(());
            }
        }

        MetricsRecorder::record_cleanup_run();

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
        let jobs_to_tag = match config
            .database()
            .get_jobs_without_storage_artifacts_tagged(Some(STORAGE_CLEANUP_MAX_JOBS_PER_RUN as i64))
            .await
        {
            Ok(jobs) => jobs,
            Err(e) => {
                MetricsRecorder::record_cleanup_failure("db_jobs_lookup");
                return Err(e.into());
            }
        };

        if jobs_to_tag.is_empty() {
            debug!("No completed StateTransition jobs need artifact tagging");
            return Ok(());
        }

        debug!("Found {} completed StateTransition jobs that need artifact tagging", jobs_to_tag.len());

        let total_to_process = jobs_to_tag.len();
        let mut jobs_processed = 0;
        let mut total_artifacts_tagged = 0;

        for job in jobs_to_tag {
            MetricsRecorder::record_cleanup_job_attempted();
            let job_id = job.internal_id;

            let state_metadata: StateUpdateMetadata = match job.metadata.specific.clone().try_into() {
                Ok(m) => m,
                Err(e) => {
                    MetricsRecorder::record_cleanup_failure("metadata_parse");
                    error!(job_id = %job_id, error = %e, "Failed to parse StateUpdateMetadata");
                    continue;
                }
            };

            let paths_to_tag = match self.collect_artifact_paths(config, job_id, &state_metadata).await {
                Ok(paths) => paths,
                Err(e) => {
                    error!(job_id = %job_id, error = %e, "Failed to collect artifacts for cleanup");
                    continue;
                }
            };

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

            MetricsRecorder::record_cleanup_artifacts_tagged(tagged_count as f64);

            if !all_tagged {
                MetricsRecorder::record_cleanup_failure("tagging");
                warn!(
                    job_id = %job_id,
                    tagged = tagged_count,
                    total = paths_to_tag.len(),
                    "Partial tagging failure, will retry next run"
                );
                continue;
            }

            if let Err(e) = self.mark_job_tagged(config, &job, state_metadata).await {
                MetricsRecorder::record_cleanup_failure("update_job");
                error!(job_id = %job_id, error = %e, "Failed to update job metadata after tagging");
                continue;
            }

            jobs_processed += 1;
            total_artifacts_tagged += tagged_count;
            MetricsRecorder::record_cleanup_job_processed();
            debug!(job_id = %job_id, artifact_count = tagged_count, "Successfully tagged artifacts for expiration");
        }

        info!(
            "Storage cleanup summary: processed {}/{} jobs, tagged {} artifacts total",
            jobs_processed, total_to_process, total_artifacts_tagged
        );

        Ok(())
    }

    /// Collect artifact paths from:
    /// - known batch-scoped artifacts (no directory listing)
    /// - known blob files (0..=MAX_BLOBS)
    /// - SNOS batch root artifacts (L2: from DB, L3: job_id)
    /// - state update file
    async fn collect_artifact_paths(
        &self,
        config: &Arc<Config>,
        job_id: u64,
        state_metadata: &StateUpdateMetadata,
    ) -> color_eyre::Result<Vec<String>> {
        let mut paths = BTreeSet::new();

        // Batch-scoped artifacts (known filenames)
        let batch_artifact_files = [
            CAIRO_PIE_FILE_NAME,
            DA_SEGMENT_FILE_NAME,
            PROGRAM_OUTPUT_FILE_NAME,
            SNOS_OUTPUT_FILE_NAME,
            PROOF_FILE_NAME,
        ];
        for file in batch_artifact_files {
            paths.insert(get_batch_artifact_file(job_id, file));
        }

        // Blob files (0..=MAX_BLOBS). Missing files are treated as NotFound during tagging.
        for blob_index in 0..=MAX_BLOBS {
            paths.insert(get_batch_blob_file(job_id, blob_index as u64));
        }

        if matches!(state_metadata.context, SettlementContext::Batch(_)) {
            // Include SNOS batch dirs for all SNOS batches in this aggregator batch
            let snos_batches = match config.database().get_snos_batches_by_aggregator_index(job_id).await {
                Ok(batches) => batches,
                Err(e) => {
                    MetricsRecorder::record_cleanup_failure("snos_batch_lookup");
                    return Err(e.into());
                }
            };
            for snos_batch in snos_batches {
                let snos_dir = get_snos_batch_dir(snos_batch.index);
                for file in [CAIRO_PIE_FILE_NAME, SNOS_OUTPUT_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME] {
                    paths.insert(format!("{}/{}", snos_dir, file));
                }
            }
        } else {
            // L3: use job_id as SNOS batch id (assumes block_number == snos_batch_id)
            let snos_dir = get_snos_batch_dir(job_id);
            for file in [
                CAIRO_PIE_FILE_NAME,
                SNOS_OUTPUT_FILE_NAME,
                PROGRAM_OUTPUT_FILE_NAME,
                BLOB_DATA_FILE_NAME,
                PROOF_FILE_NAME,
                PROOF_PART2_FILE_NAME,
            ] {
                paths.insert(format!("{}/{}", snos_dir, file));
            }
        }

        // Add state update file (single file, always included)
        paths.insert(get_batch_state_update_file(job_id));
        Ok(paths.into_iter().collect())
    }

    /// Tag artifacts for expiration. Returns (tagged_count, all_succeeded).
    /// Handles non-existent files gracefully (counts as success).
    async fn tag_artifacts(&self, config: &Arc<Config>, job_id: u64, paths: &[String]) -> (usize, bool) {
        // Create tags once - using &str avoids allocations
        let tags = [(STORAGE_EXPIRATION_TAG_KEY, STORAGE_EXPIRATION_TAG_VALUE)];
        let mut tagged_count = 0;

        for path in paths {
            match config.storage().tag_object(path, &tags).await {
                Ok(_) => {
                    tagged_count += 1;
                    debug!(job_id = %job_id, path = %path, "Tagged artifact for expiration");
                }
                Err(e) => match e {
                    StorageError::NotFound(_) => {
                        // Handle non-existent files gracefully (file may have been deleted)
                        debug!(job_id = %job_id, path = %path, "Artifact not found, skipping");
                        tagged_count += 1; // Count as success since there's nothing to tag
                    }
                    _ => {
                        error!(job_id = %job_id, path = %path, error = %e, "Failed to tag artifact");
                        return (tagged_count, false);
                    }
                },
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
