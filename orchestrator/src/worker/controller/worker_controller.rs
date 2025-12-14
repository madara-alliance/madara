use crate::core::config::Config;
use crate::error::event::EventSystemError;
use crate::error::event::EventSystemResult;
use crate::types::queue::QueueType;
use crate::types::Layer;
use crate::worker::controller::event_worker::EventWorker;

use futures::future::try_join_all;
use std::sync::Arc;
use std::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, info_span, warn, Instrument};

#[derive(Clone)]
pub struct WorkerController {
    config: Arc<Config>,
    workers: Arc<Mutex<Vec<Arc<EventWorker>>>>,
    cancellation_token: CancellationToken,
}

impl WorkerController {
    /// new - Create a new WorkerController
    /// This function creates a new WorkerController with the given configuration
    /// It returns a new WorkerController instance
    /// # Arguments
    /// * `config` - The configuration for the WorkerController
    /// * `cancellation_token` - Token for coordinated shutdown
    /// # Returns
    /// * `WorkerController` - A new WorkerController instance
    pub fn new(config: Arc<Config>, cancellation_token: CancellationToken) -> Self {
        Self { config, workers: Arc::new(Mutex::new(Vec::new())), cancellation_token }
    }

    /// workers - Get the list of workers
    /// This function returns the list of workers
    /// # Returns
    /// * `Vec<Arc<EventWorker>>` - A list of workers
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub fn workers(&self) -> EventSystemResult<Vec<Arc<EventWorker>>> {
        let workers = self.workers.lock().map_err(|e| EventSystemError::MutexPoisonError(e.to_string()))?;
        Ok(workers.clone())
    }

    /// run - Run the WorkerController
    /// This function starts the WorkerController and spawns event workers for each queue type
    /// It waits for all workers to complete, which typically means it runs indefinitely until shutdown
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run(&self) -> EventSystemResult<()> {
        let queues = match self.config.layer() {
            Layer::L2 => Self::get_l2_queues(),
            Layer::L3 => Self::get_l3_queues(),
        };
        let mut worker_set = tokio::task::JoinSet::new();
        for queue_type in queues.into_iter() {
            let queue_type = queue_type.clone();
            let self_clone = self.clone();
            worker_set.spawn(async move { self_clone.create_span(&queue_type).await });
        }
        // Since there is no support to join all in futures, we need to join each worker one by one
        while let Some(result) = worker_set.join_next().await {
            // If any worker fails (initialization or infrastructure error), propagate the error
            // This will trigger application shutdown
            result??;
        }
        Ok(())
    }

    /// create_event_handler - Create an event handler
    /// This function creates an event handler for the WorkerController
    /// It returns a Result<Arc<EventWorker>, EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<Arc<EventWorker>, EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    async fn create_event_handler(&self, queue_type: &QueueType) -> EventSystemResult<Arc<EventWorker>> {
        // Create a child token for this specific worker
        let worker_token = self.cancellation_token.child_token();
        let worker = Arc::new(EventWorker::new(queue_type.clone(), self.config.clone(), worker_token)?);

        // Fix: Properly store the worker by modifying the vector directly
        let mut workers = self.workers.lock().map_err(|e| EventSystemError::MutexPoisonError(e.to_string()))?;
        workers.push(worker.clone());
        drop(workers); // Release the lock explicitly

        Ok(worker)
    }

    /// get_l2_queues - Get the list of queues for L2 network
    /// This function returns a list of queues for L2 network
    /// # Returns
    /// * `Vec<QueueType>` - A list of queues for L2 network
    fn get_l2_queues() -> Vec<QueueType> {
        vec![
            // QueueType::SnosJobProcessing,
            // QueueType::ProvingJobProcessing,
            // QueueType::AggregatorJobProcessing,
            // QueueType::UpdateStateJobProcessing,
            // QueueType::SnosJobVerification,
            // QueueType::ProvingJobVerification,
            // QueueType::AggregatorJobVerification,
            // QueueType::UpdateStateJobVerification,
            QueueType::WorkerTrigger,
            QueueType::JobHandleFailure,
        ]
    }

    /// get_l3_queues - Get the list of queues for L3 network
    /// This function returns a list of queues for L3 network
    /// # Returns
    /// * `Vec<QueueType>` - A list of queues for L3 network
    fn get_l3_queues() -> Vec<QueueType> {
        vec![
            QueueType::SnosJobProcessing,
            QueueType::ProvingJobProcessing,
            QueueType::ProofRegistrationJobProcessing,
            QueueType::DataSubmissionJobProcessing,
            QueueType::UpdateStateJobProcessing,
            QueueType::SnosJobVerification,
            QueueType::ProvingJobVerification,
            QueueType::ProofRegistrationJobVerification,
            QueueType::DataSubmissionJobVerification,
            QueueType::UpdateStateJobVerification,
            QueueType::WorkerTrigger,
            QueueType::JobHandleFailure,
        ]
    }

    /// Create a span for a queue type.
    /// This function creates a span for a queue type and starts a worker for that queue.
    /// It returns a `Result<(), EventSystemError>` indicating whether the operation was successful or not.
    /// Note that this is a blocking function. It'll return only
    /// 1. There is a graceful shutdown (`Ok` in this case)
    /// 2. There is an error in the queue message handler (`Err` in this case)
    ///
    /// It does the following things:
    /// 1. Create an info span for the given queue
    /// 2. Create an event handler for the queue for handling the actual messages using [create_event_handler](#method.create_event_handler)
    /// 3. Start the handler for starting message consumption
    ///
    /// # Arguments
    /// * `q` - The queue type for which to create a span
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    async fn create_span(&self, q: &QueueType) -> EventSystemResult<()> {
        let span = info_span!("worker", q = ?q);

        async move {
            // If worker creation fails, this is a critical initialization error
            let handler = match self.create_event_handler(q).await {
                Ok(handler) => handler,
                Err(e) => {
                    error!("🚨 Critical: Failed to create handler for queue type {:?}: {:?}", q, e);
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
                    error!("🚨 Critical: Worker for queue type {:?} failed with infrastructure error: {:?}", q, e);
                    error!("This indicates a serious system problem that requires shutdown");
                    Err(e)
                }
            }
        }
        .instrument(span)
        .await
    }

    /// shutdown - Trigger a graceful shutdown
    /// This function triggers a graceful shutdown of all workers
    /// It waits for workers to complete their current tasks and exit cleanly
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
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
