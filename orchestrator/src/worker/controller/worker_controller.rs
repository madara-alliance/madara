use crate::core::config::Config;
use crate::error::event::EventSystemError;
use crate::error::event::EventSystemResult;
use crate::types::queue::QueueType;
use crate::worker::controller::event_worker::EventWorker;

use futures::future::try_join_all;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn, Instrument};

/// WorkerController manages WorkerTrigger queue consumption
/// NOTE: Job processing is handled by workers, not SQS
#[derive(Clone)]
pub struct WorkerController {
    config: Arc<Config>,
    workers: Arc<Mutex<Vec<Arc<EventWorker>>>>,
    cancellation_token: CancellationToken,
}

impl WorkerController {
    /// new - Create a new WorkerController for WorkerTrigger queue
    pub fn new(config: Arc<Config>, cancellation_token: CancellationToken) -> Self {
        Self { config, workers: Arc::new(Mutex::new(Vec::new())), cancellation_token }
    }

    /// workers - Get the list of workers
    pub fn workers(&self) -> EventSystemResult<Vec<Arc<EventWorker>>> {
        let workers = self.workers.lock().map_err(|e| EventSystemError::MutexPoisonError(e.to_string()))?;
        Ok(workers.clone())
    }

    /// run - Run the WorkerController (only WorkerTrigger queue)
    pub async fn run(&self) -> EventSystemResult<()> {
        // Only spawn WorkerTrigger worker - job processing uses worker mode
        let queue_type = QueueType::WorkerTrigger;

        info!("Starting WorkerController with WorkerTrigger queue only (job processing uses worker mode)");

        self.create_span(&queue_type).await
    }

    /// create_event_handler - Create a WorkerTrigger event handler
    async fn create_event_handler(&self, queue_type: &QueueType) -> EventSystemResult<Arc<EventWorker>> {
        // Create a child token for this specific worker
        let worker_token = self.cancellation_token.child_token();
        let worker = Arc::new(EventWorker::new(queue_type.clone(), self.config.clone(), worker_token)?);

        // Store the worker
        let mut workers = self.workers.lock().map_err(|e| EventSystemError::MutexPoisonError(e.to_string()))?;
        workers.push(worker.clone());
        drop(workers); // Release the lock explicitly

        Ok(worker)
    }

    /// Create a span for the WorkerTrigger queue and start the worker
    async fn create_span(&self, q: &QueueType) -> EventSystemResult<()> {
        let span = info_span!("worker", q = ?q);

        async move {
            // If worker creation fails, this is a critical initialization error
            let handler = match self.create_event_handler(q).await {
                Ok(handler) => handler,
                Err(e) => {
                    error!("Critical: Failed to create handler for queue type {:?}: {:?}", q, e);
                    error!("This is a worker initialization error that requires system shutdown");
                    return Err(e);
                }
            };

            // Run the worker - job processing errors within the worker are handled internally
            // Only infrastructure/initialization errors should propagate out
            match handler.run().await {
                Ok(_) => {
                    // Worker completed unexpectedly - this should not happen in normal operation
                    // since workers run infinite loops. This indicates a problem.
                    warn!("Worker for queue type {:?} completed unexpectedly (this is normal during shutdown)", q);
                    Ok(())
                }
                Err(e) => {
                    error!("Critical: Worker for queue type {:?} failed with infrastructure error: {:?}", q, e);
                    error!("This indicates a serious system problem that requires shutdown");
                    Err(e)
                }
            }
        }
        .instrument(span)
        .await
    }

    /// shutdown - Trigger a graceful shutdown
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        info!("Initiating WorkerController graceful shutdown");

        // Signal all workers to shut down gracefully
        let workers = self.workers()?;
        info!("Signaling {} workers to shutdown gracefully", workers.len());

        let futures: Vec<_> = workers.iter().map(|worker| worker.shutdown()).collect();
        info!("All workers signaled to shutdown - they will complete current tasks and exit");

        try_join_all(futures).await?;
        info!("WorkerController shutdown completed");
        Ok(())
    }
}
