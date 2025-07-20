use crate::core::config::Config;
use crate::error::event::EventSystemResult;
use crate::types::queue::QueueType;
use crate::types::Layer;
use crate::worker::controller::event_worker::EventWorker;
use anyhow::anyhow;

use std::sync::Arc;
use tracing::{info, info_span};

#[derive(Clone)]
pub struct WorkerController {
    config: Arc<Config>,
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
        Self { config }
    }

    /// run - Run the WorkerController
    /// This function runs the WorkerController and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
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
            worker_set.spawn(async move {
                self_clone.create_span(&queue_type).await;
            });
        }
        // since there is not support to join all in futures, we need to join each worker one by one
        while let Some(result) = worker_set.join_next().await {
            result?;
        }
        Ok(())
    }

    /// create_event_handler - Create an event handler
    /// This function creates an event handler for the WorkerController
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    async fn create_event_handler(&self, queue_type: &QueueType) -> EventSystemResult<Arc<EventWorker>> {
        Ok(Arc::new(EventWorker::new(queue_type.clone(), self.config.clone())?))
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
            QueueType::AggregatorJobProcessing,
            QueueType::UpdateStateJobProcessing,
            QueueType::SnosJobVerification,
            QueueType::ProvingJobVerification,
            QueueType::DataSubmissionJobVerification,
            QueueType::AggregatorJobVerification,
            QueueType::UpdateStateJobVerification,
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

    #[tracing::instrument(skip(self), fields(q = %q))]
    async fn create_span(&self, q: &QueueType) {
        let span = info_span!("worker", q = ?q);
        let _guard = span.enter();
        info!("Starting worker for queue type {:?}", q);
        match self.create_event_handler(q).await {
            Ok(handler) => match handler.run().await {
                Ok(_) => info!("Worker for queue type {:?} is completed", q),
                Err(e) => {
                    let _ = anyhow!("🚨Failed to start worker: {:?}", e);
                    tracing::error!("🚨Failed to start worker: {:?}", e)
                }
            },
            Err(e) => tracing::error!("🚨Failed to create handler: {:?}", e),
        };
    }

    /// trigger_graceful_shutdown - Trigger a graceful shutdown
    /// This function triggers a graceful shutdown of the WorkerController
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn trigger_graceful_shutdown(&self) -> EventSystemResult<()> {
        Ok(())
    }
}
