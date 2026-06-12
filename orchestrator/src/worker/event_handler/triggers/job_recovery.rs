use crate::cli::service::DEFAULT_TIMEOUT_SECONDS;
use crate::core::config::Config;
use crate::types::constant::ORCHESTRATOR_VERSION;
use crate::types::jobs::job_item::JobItem;
use crate::types::jobs::types::JobStatus;
use crate::utils::metrics_recorder::MetricsRecorder;
use crate::worker::event_handler::factory::factory;
use crate::worker::event_handler::triggers::JobTrigger;
use crate::worker::service::JobService;
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Maximum number of jobs re-enqueued per status group per sweep. Anything past
/// the limit is picked up by the next sweep.
const RECOVERY_BATCH_LIMIT: i64 = 100;

/// Headroom added on top of a job's own timeout before it is considered
/// stranded, covering the queue visibility window (`queue_control` sizes the
/// visibility timeout as job timeout + 60s).
const RECOVERY_VISIBILITY_MARGIN_SECONDS: u64 = DEFAULT_TIMEOUT_SECONDS + 60;

/// Re-enqueues jobs whose driving queue message was lost.
///
/// Every queue-driven transition writes the job to the database *before*
/// enqueueing the message that drives the next step (`create_job`,
/// `process_job` -> verify queue, verification retry -> process queue). If the
/// enqueue fails after the database write, the job sits in `Created`,
/// `PendingVerification`, `VerificationFailed` or `PendingRetry` forever: the
/// in-consumer self-healing only runs when a message for the job arrives, which
/// is exactly what never happens.
///
/// A job in one of those states with a live message is always touched within a
/// bounded window: consuming the message updates the job (bumping
/// `updated_at`), and an unconsumed message is redelivered within the queue
/// visibility timeout. So a job whose `updated_at` is older than its own
/// timeout plus the visibility window has no message driving it, and it is safe
/// to enqueue a fresh one. Duplicates are harmless by design: `process_job` and
/// `verify_job` ack messages for jobs that are already past the expected state.
pub struct JobRecoveryTrigger;

#[async_trait]
impl JobTrigger for JobRecoveryTrigger {
    /// Recovery must keep running when the pipeline is halted by a `Failed` /
    /// `VerificationTimeout` job: it creates no new work, it only lets
    /// already-created jobs drain.
    async fn is_worker_enabled(&self, _config: Arc<Config>) -> color_eyre::Result<bool> {
        Ok(true)
    }

    async fn run_worker(&self, config: Arc<Config>) -> color_eyre::Result<()> {
        let processable = config
            .database()
            .get_jobs_by_types_and_statuses(
                vec![],
                vec![JobStatus::Created, JobStatus::VerificationFailed, JobStatus::PendingRetry],
                Some(RECOVERY_BATCH_LIMIT),
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;
        for job in processable {
            if !Self::is_stranded(&job, config.service_config().get_job_timeout(&job.job_type)) {
                continue;
            }
            Self::record_stranded(&job);
            match JobService::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await {
                Ok(()) => Self::record_recovered(&job, "process"),
                Err(e) => warn!(job_id = %job.id, error = ?e, "Failed to re-enqueue stranded job for processing"),
            }
        }

        let pending_verification = config
            .database()
            .get_jobs_by_types_and_statuses(
                vec![],
                vec![JobStatus::PendingVerification],
                Some(RECOVERY_BATCH_LIMIT),
                Some(ORCHESTRATOR_VERSION.to_string()),
            )
            .await?;
        for job in pending_verification {
            let polling_delay = factory::get_job_handler(&job.job_type).await.verification_polling_delay_seconds();
            if !Self::is_stranded(&job, polling_delay) {
                continue;
            }
            Self::record_stranded(&job);
            match JobService::add_job_to_verify_queue(
                config.clone(),
                job.id,
                &job.job_type,
                Some(Duration::from_secs(polling_delay)),
            )
            .await
            {
                Ok(()) => Self::record_recovered(&job, "verify"),
                Err(e) => warn!(job_id = %job.id, error = ?e, "Failed to re-enqueue stranded job for verification"),
            }
        }

        Ok(())
    }
}

impl JobRecoveryTrigger {
    fn is_stranded(job: &JobItem, base_timeout_seconds: u64) -> bool {
        let threshold = base_timeout_seconds + RECOVERY_VISIBILITY_MARGIN_SECONDS;
        let stranded = job.updated_at < Utc::now() - chrono::Duration::seconds(threshold as i64);
        if !stranded {
            debug!(job_id = %job.id, status = ?job.status, "Job within liveness window, not stranded");
        }
        stranded
    }

    fn record_stranded(job: &JobItem) {
        warn!(
            job_id = %job.id,
            job_type = ?job.job_type,
            internal_id = %job.internal_id,
            status = ?job.status,
            updated_at = %job.updated_at,
            "Found stranded job with no driving queue message, re-enqueueing"
        );
        MetricsRecorder::record_orphaned_job(job);
    }

    fn record_recovered(job: &JobItem, queue: &str) {
        info!(
            job_id = %job.id,
            job_type = ?job.job_type,
            internal_id = %job.internal_id,
            "Re-enqueued stranded job to {queue} queue"
        );
        MetricsRecorder::record_healed_job(&job.job_type);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::jobs::external_id::ExternalId;
    use crate::types::jobs::metadata::{CommonMetadata, JobMetadata, JobSpecificMetadata, SnosMetadata};
    use crate::types::jobs::types::JobType;
    use uuid::Uuid;

    fn job_updated_secs_ago(secs: i64) -> JobItem {
        let ts = Utc::now() - chrono::Duration::seconds(secs);
        JobItem {
            id: Uuid::new_v4(),
            internal_id: 1,
            job_type: JobType::SnosRun,
            status: JobStatus::Created,
            external_id: ExternalId::Number(0),
            metadata: JobMetadata {
                common: CommonMetadata::default(),
                specific: JobSpecificMetadata::Snos(SnosMetadata::default()),
            },
            version: 0,
            created_at: ts,
            updated_at: ts,
        }
    }

    #[test]
    fn job_inside_liveness_window_is_not_stranded() {
        // updated_at is younger than timeout + visibility margin
        let job = job_updated_secs_ago(30);
        assert!(!JobRecoveryTrigger::is_stranded(&job, 300));
    }

    #[test]
    fn job_just_past_timeout_is_still_covered_by_visibility_margin() {
        let job = job_updated_secs_ago(310);
        assert!(!JobRecoveryTrigger::is_stranded(&job, 300));
    }

    #[test]
    fn job_past_timeout_and_margin_is_stranded() {
        let job = job_updated_secs_ago(300 + RECOVERY_VISIBILITY_MARGIN_SECONDS as i64 + 10);
        assert!(JobRecoveryTrigger::is_stranded(&job, 300));
    }
}
