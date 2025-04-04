use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
use uuid::Uuid;

use crate::config::Config;
use crate::jobs::types::JobItem;
use crate::jobs::JobError;
use crate::queue::job_queue::add_job_to_process_queue;

#[derive(Default)]
pub struct ProcessingLocks {
    pub snos_job_processing_lock: Option<Arc<JobProcessingState>>,
    pub proving_job_processing_lock: Option<Arc<JobProcessingState>>,
}

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
                add_job_to_process_queue(job.id, &job.job_type, config.clone()).await?;
                Err(JobError::MaxCapacityReached)
            }
            Ok(Err(e)) => Err(JobError::LockError(e.to_string())),
        }
    }

    pub async fn try_release_lock<'a>(&'a self, permit: SemaphorePermit<'a>, job_id: &Uuid) -> Result<(), JobError> {
        let mut active_jobs = self.active_jobs.lock().await;
        active_jobs.remove(job_id);
        drop(active_jobs); // Explicitly drop the lock (optional but clear)
        drop(permit); // Explicitly drop the permit (optional but clear)
        Ok(())
    }
}
