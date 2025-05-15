use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use uuid::Uuid;

use crate::core::config::Config;
use crate::error::job::JobError;
use crate::types::jobs::job_item::JobItem;
use crate::worker::service::JobService;

/// wait_until_ready - Wait until the provided function returns a result or the timeout is reached
/// This function will repeatedly call the provided function until it returns a result or the timeout is reached
/// It will return the result of the function or an error if the timeout is reached
/// # Arguments
/// * `f` - The function to call
/// * `timeout_secs` - The timeout in seconds
/// # Returns
/// * `Result<T, E>` - The result of the function or an error if the timeout is reached
/// # Examples
/// ```
/// use std::time::Duration;
/// use orchestrator::utils::helpers::wait_until_ready;
///     
/// async fn wait_for_ready() -> Result<(), Box<dyn std::error::Error>> {
///     let result = wait_until_ready(|| async { Ok(()) }, 10).await;
///     assert!(result.is_ok());
///     Ok(())
/// }
/// ```
pub async fn wait_until_ready<F, T, E>(mut f: F, timeout_secs: u64) -> Result<T, E>
where
    F: FnMut() -> Pin<Box<dyn Future<Output = Result<T, E>> + Send>>,
{
    let start = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    loop {
        match f().await {
            Ok(val) => return Ok(val),
            Err(_) if start.elapsed() < timeout => {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[derive(Default)]
pub struct ProcessingLocks {
    pub snos_job_processing_lock: Option<Arc<JobProcessingState>>,
    pub proving_job_processing_lock: Option<Arc<JobProcessingState>>,
}

/// JobProcessingState is a struct that holds the state of the job processing lock
/// It is used to limit a job been get dupplicated in multiple place
/// It uses a semaphore to limit been getting
/// It also uses a mutex to hold the set of active jobs
pub struct JobProcessingState {
    pub semaphore: Semaphore,
    pub active_jobs: Mutex<HashSet<Uuid>>,
}
impl JobProcessingState {
    pub fn new(max_parallel_jobs: usize) -> Self {
        JobProcessingState { semaphore: Semaphore::new(max_parallel_jobs), active_jobs: Mutex::new(HashSet::new()) }
    }

    pub async fn get_active_jobs(&self) -> HashSet<Uuid> {
        self.active_jobs.lock().await.clone()
    }

    pub fn get_available_permits(&self) -> usize {
        self.semaphore.available_permits()
    }

    pub async fn try_acquire_lock<'a>(
        &'a self,
        job: &JobItem,
        config: Arc<Config>,
    ) -> Result<SemaphorePermit<'a>, JobError> {
        // Trying to acquire permit with a timeout.
        match tokio::time::timeout(Duration::from_millis(100), self.semaphore.acquire()).await {
            Ok(Ok(permit)) => {
                {
                    let mut active_jobs = self.active_jobs.lock().await;
                    active_jobs.insert(job.id);
                    drop(active_jobs);
                }
                tracing::info!(job_id = %job.id, "Job {} acquired lock", job.id);
                Ok(permit)
            }
            Err(_) => {
                tracing::error!(job_id = %job.id, "Job {} waiting - at max capacity ({} available permits)", job.id, self.get_available_permits());
                JobService::add_job_to_process_queue(job.id, &job.job_type, config.clone()).await?;
                Err(JobError::MaxCapacityReached)
            }
            Ok(Err(e)) => Err(JobError::LockError(e.to_string())),
        }
    }

    pub async fn try_release_lock<'a>(&'a self, permit: SemaphorePermit<'a>) -> Result<(), JobError> {
        drop(permit); // Explicitly drop the permit (optional but clear)
        Ok(())
    }
}
