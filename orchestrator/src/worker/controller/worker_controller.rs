use crate::core::config::Config;
use crate::error::event::EventSystemError;
use crate::error::event::EventSystemResult;
use crate::types::queue::QueueType;
use crate::worker::controller::event_worker::EventWorker;
use color_eyre::eyre::eyre;
use futures::future::try_join_all;
use tracing::info_span;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::info;

#[derive(Clone)]
pub struct WorkerController {
    config: Arc<Config>,
    workers: Arc<Mutex<Vec<Arc<EventWorker>>>>,
}

impl WorkerController {
    /// new - Create a new WorkerController
    /// This function creates a new WorkerController with the given configuration
    /// It returns a new WorkerController instance
    /// # Arguments
    /// * `config` - The configuration for the WorkerController
    /// # Returns
    /// * `WorkerController` - A new WorkerController instance
    pub fn new(config: Arc<Config>) -> Self {
        Self { config, workers: Arc::new(Mutex::new(Vec::new())) }
    }

    pub fn workers(&self) -> EventSystemResult<Vec<Arc<EventWorker>>> {
        let workers = self.workers.lock().map_err(|e| EventSystemError::MutexPoisonError(e.to_string()))?;
        Ok(workers.clone())
    }

    /// run - Run the WorkerController
    /// This function runs the WorkerController and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run(&self) -> EventSystemResult<()> {
        self.run_l2().await
    }

    /// create_event_handler - Create an event handler
    /// This function creates an event handler for the WorkerController
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    async fn create_event_handler(&self, queue_type: &QueueType) -> EventSystemResult<Arc<EventWorker>> {
        let worker = Arc::new(EventWorker::new(queue_type.clone(), self.config.clone())?);
        self.workers()?.push(worker.clone());
        Ok(worker)
    }

    /// get_l2_queues - Get the list of queues for L2 network
    /// This function returns a list of queues for L2 network
    /// # Returns
    /// * `Vec<QueueType>` - A list of queues for L2 network
    fn get_l2_queues() -> Vec<QueueType> {
        vec![
            QueueType::SnosJobProcessing,
            QueueType::ProvingJobProcessing,
            QueueType::DataSubmissionJobProcessing,
            QueueType::UpdateStateJobProcessing,
            QueueType::SnosJobVerification,
            QueueType::ProvingJobVerification,
            QueueType::DataSubmissionJobVerification,
            QueueType::UpdateStateJobVerification,
            QueueType::WorkerTrigger,
            QueueType::JobHandleFailure,
        ]
    }

    /// run_l2 - Run the WorkerController for L2 Madara Network
    /// This function runs the WorkerController for L2 and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run_l2(&self) -> EventSystemResult<()> {
        for queue_type in Self::get_l2_queues().into_iter() {
            let queue_type = queue_type.clone();
            let self_clone = self.clone();
            tokio::spawn(async move {
                if let Err(e) = self_clone.create_span(&queue_type).await {
                    tracing::error!("Worker span failed: {:?}", e);
                    panic!("Worker span failed: {:?}", e);
                }
            });
        }
        Ok(())
    }
    
    async fn create_span(&self, q: &QueueType) -> EventSystemResult<()>  {
        let span_clone = info_span!("worker", q = ?q);
        let _guard = span_clone.enter();
        info!("Starting worker for queue type {:?}", q);
        match self.create_event_handler(q).await {
            Ok(handler) => match handler.run().await {
                Ok(_) => info!("Worker for queue type {:?} is completed", q),
                Err(e) => {
                    tracing::error!("🚨Failed to start worker: {:?}", e);
                    return Err(e);
                }
            },
            Err(e) => {
                tracing::error!("🚨Failed to create handler: {:?}", e);
                return Err(e);
            },
        }
        Ok(())
    }

    /// run_l3 - Run the WorkerController for L3 Madara Network
    /// This function runs the WorkerController for L3 and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run_l3(&self) -> EventSystemResult<()> {
        Ok(())
    }

    /// shutdown - Trigger a graceful shutdown
    /// This function triggers a graceful shutdown of the WorkerController
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        let workers = self.workers()?;
        let futures: Vec<_> = workers.iter().map(|worker| worker.shutdown()).collect();
        try_join_all(futures).await?;
        Ok(())
    }
}
