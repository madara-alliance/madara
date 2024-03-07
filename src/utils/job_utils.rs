use crate::config::Config;
use crate::jobs::types::JobItem;
use color_eyre::eyre::eyre;
use color_eyre::Result;
use tracing::log;
use uuid::Uuid;

pub async fn get_job_or_log_error(config: &Config, id: Uuid) -> Result<JobItem> {
    let job = config.database().get_job_by_id(id).await?;
    let job = match job {
        Some(job) => job,
        None => {
            log::error!("Job with id {} not found when processing", id);
            return Err(eyre!("Job with id {} not found when processing", id));
        }
    };
    Ok(job)
}
