use tracing::warn;

use crate::types::jobs::job_item::JobItem;

pub mod constants;
pub mod helpers;
pub mod instrument;
pub mod job_status_metrics;
pub mod logging;
pub mod metrics;
pub mod metrics_recorder;
pub mod rest_client;
pub mod signal_handler;

/// TODO: This is super Awkward to have this code here
/// but will try to remove this and move it to the config from the root path
pub const COMPILED_OS: &[u8] = include_bytes!("../../../build-artifacts/cairo_lang/os_latest.json");

/// Filter jobs to only include those with compatible orchestrator versions.
/// - Jobs without orchestrator_version (legacy jobs) are included for backward compatibility
/// - Jobs with matching orchestrator version are included
/// - Jobs with incompatible versions are filtered out with a warning
pub fn filter_jobs_by_orchestrator_version(jobs: Vec<JobItem>) -> Vec<JobItem> {
    let current_orchestrator_version = crate::types::constant::ORCHESTRATOR_VERSION;
    jobs.into_iter()
        .filter(|job| match &job.metadata.common.orchestrator_version {
            Some(version) if version != current_orchestrator_version => {
                warn!(
                    job_id = %job.id,
                    job_version = %version,
                    current_version = %current_orchestrator_version,
                    "Skipping job with incompatible orchestrator version"
                );
                false
            }
            _ => true,
        })
        .collect()
}
