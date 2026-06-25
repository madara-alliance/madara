pub(crate) mod aggregator;
pub(crate) mod aggregator_batching;
pub(crate) mod batching;
pub(crate) mod data_submission_worker;
pub(crate) mod proof_registration;
pub(crate) mod proving;
pub(crate) mod snos;
pub(crate) mod snos_batching;
pub(crate) mod storage_cleanup;
pub(crate) mod update_state;

use crate::core::config::Config;
use crate::types::constant::ORCHESTRATOR_VERSION;
use crate::types::jobs::types::{JobStatus, JobType};
use async_trait::async_trait;
use std::sync::Arc;

/// Pure helper that computes the buffer slots available for new job creation
/// given the current `[oldest-incomplete, latest]` window.
///
/// Split out from [`calculate_jobs_to_create`] so the math is directly unit-
/// testable without mocking the database trait.
fn compute_buffer_slots(
    latest_internal_id: Option<u64>,
    oldest_incomplete_internal_id: Option<u64>,
    buffer_size: u64,
) -> u64 {
    match (latest_internal_id, oldest_incomplete_internal_id) {
        (Some(l), Some(o)) => buffer_size.saturating_sub(l.saturating_sub(o) + 1),
        // No jobs, or all completed — can fill the entire buffer.
        _ => buffer_size,
    }
}

/// Computes how many new jobs of `job_type` can be created without exceeding
/// `buffer_size`, based on the window between the oldest non-completed job and
/// the latest job of that type.
///
/// The buffer mechanism prevents a stalled low-internal-id job from letting the
/// pipeline run away ahead of it: newer jobs can be created until the window
/// (latest - oldest_incomplete + 1) hits `buffer_size`, at which point creation
/// pauses until the stall clears. Critical for job types whose downstream
/// consumers must run in strict internal_id order (e.g. state updates).
///
/// Shared by [`snos::SnosJobTrigger`] and [`aggregator::AggregatorJobTrigger`];
/// any trigger whose `internal_id` is monotonically increasing can reuse it.
pub(crate) async fn calculate_jobs_to_create(
    config: &Arc<Config>,
    job_type: JobType,
    buffer_size: u64,
) -> color_eyre::Result<u64> {
    let latest = config
        .database()
        .get_latest_job_by_type(job_type.clone(), Some(ORCHESTRATOR_VERSION.to_string()))
        .await?
        .map(|j| j.internal_id);

    let oldest_incomplete = config
        .database()
        .get_oldest_job_by_type_excluding_statuses(
            job_type,
            vec![JobStatus::Completed],
            Some(ORCHESTRATOR_VERSION.to_string()),
        )
        .await?
        .map(|j| j.internal_id);

    Ok(compute_buffer_slots(latest, oldest_incomplete, buffer_size))
}

#[async_trait]
pub trait JobTrigger: Send + Sync {
    async fn run_worker_if_enabled(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        if !self.is_worker_enabled(config.clone()).await? {
            return Ok(());
        }
        self.run_worker(config).await
    }

    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()>;

    // Assumption
    // If say a job for block X fails, we don't want the worker to respawn another job for the same
    // block, we will resolve the existing failed job first.

    // We assume the system to keep working till a job hasn't failed.
    // As soon as it fails, we currently halt any more execution and wait for manual intervention.

    // Checks if any of the jobs have failed (empty job_type vector implies any job)
    // Failure: JobStatus::VerificationFailed, JobStatus::VerificationTimeout, JobStatus::Failed
    // Halts any new job creation till all the count of failed jobs is not Zero.
    async fn is_worker_enabled(&self, config: Arc<Config>) -> color_eyre::Result<bool> {
        let failed_jobs = config
            .database()
            .get_jobs_by_types_and_statuses(
                vec![],
                vec![JobStatus::Failed, JobStatus::VerificationTimeout],
                None,
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;

        if !failed_jobs.is_empty() {
            // Group jobs by status to provide accurate logging
            let failed_count = failed_jobs.iter().filter(|j| j.status == JobStatus::Failed).count();
            let timeout_count = failed_jobs.iter().filter(|j| j.status == JobStatus::VerificationTimeout).count();

            let status_summary = match (failed_count > 0, timeout_count > 0) {
                (true, true) => format!("{} Failed, {} VerificationTimeout", failed_count, timeout_count),
                (true, false) => format!("{} Failed", failed_count),
                (false, true) => format!("{} VerificationTimeout", timeout_count),
                (false, false) => "0".to_string(),
            };

            tracing::warn!(
                "There are {} jobs in the DB ({}). Not creating new jobs to prevent inconsistencies (existing jobs will be processed). Please manually fix the failed jobs before continuing!",
                failed_jobs.len(),
                status_summary
            );
            return Ok(false);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::compute_buffer_slots;
    use rstest::rstest;

    #[rstest]
    // No jobs at all → full buffer available.
    #[case(None, None, 5, 5)]
    // Latest exists but no incomplete (all completed) → full buffer.
    #[case(Some(10), None, 5, 5)]
    // Window [10, 12] → 3 in flight, buffer 5 → 2 slots left.
    #[case(Some(12), Some(10), 5, 2)]
    // Window [10, 10] → 1 in flight, buffer 5 → 4 slots left.
    #[case(Some(10), Some(10), 5, 4)]
    // Window [10, 15] → 6 in flight, over buffer of 5 → saturates to 0.
    #[case(Some(15), Some(10), 5, 0)]
    // Window [10, 100] → 91 in flight, buffer 5 → 0 (saturating).
    #[case(Some(100), Some(10), 5, 0)]
    // Edge: buffer_size of 0 always returns 0.
    #[case(None, None, 0, 0)]
    #[case(Some(10), Some(10), 0, 0)]
    fn compute_buffer_slots_cases(
        #[case] latest: Option<u64>,
        #[case] oldest_incomplete: Option<u64>,
        #[case] buffer_size: u64,
        #[case] expected: u64,
    ) {
        assert_eq!(compute_buffer_slots(latest, oldest_incomplete, buffer_size), expected);
    }
}
