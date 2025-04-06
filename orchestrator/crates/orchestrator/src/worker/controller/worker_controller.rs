use crate::core::config::Config;
use crate::error::event::{EventSystemError, EventSystemResult};
use crate::types::queue::QueueType;
use crate::types::worker::WorkerConfig;
use crate::worker::controller::event_worker::EventWorker;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct WorkerController {
    config: Arc<Config>,
    event_handlers: HashMap<QueueType, Arc<EventWorker>>,
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
        Self { config, event_handlers: HashMap::new() }
    }

    /// run - Run the WorkerController
    /// This function runs the WorkerController and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run(&self) -> EventSystemResult<()> {
        Ok(())
    }

    /// create_event_handler - Create an event handler
    /// This function creates an event handler for the WorkerController
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn create_event_handler(&mut self, queue_type: &QueueType) -> EventSystemResult<Arc<EventWorker>> {
        tracing::debug!("Creating Event Handler for Queue Type : {:?}", queue_type);
        if let Some(_) = self.event_handlers.get(queue_type) {
            return Err(EventSystemError::EventHandlerAlreadyExisting(queue_type.clone()));
        }
        let handler =
            Arc::new(EventWorker::new(queue_type.clone(), Arc::new(WorkerConfig::default()), self.config.clone()));
        self.event_handlers.insert(queue_type.clone(), handler.clone());
        Ok(handler)
    }

    /// get_l2_queues - Get the list of queues for L2 network
    /// This function returns a list of queues for L2 network
    /// # Returns
    /// * `Vec<QueueType>` - A list of queues for L2 network
    fn get_l2_queues() -> Vec<QueueType> {
        vec![
            QueueType::SnosJobProcessing,
            QueueType::SnosJobVerification,
            QueueType::ProvingJobProcessing,
            QueueType::ProvingJobVerification,
            QueueType::DataSubmissionJobProcessing,
            QueueType::DataSubmissionJobVerification,
            QueueType::UpdateStateJobProcessing,
            QueueType::UpdateStateJobVerification,
            QueueType::JobHandleFailure,
            QueueType::WorkerTrigger,
        ]
    }

    /// run_l2 - Run the WorkerController for L2 Madara Network
    /// This function runs the WorkerController for L2 and handles events
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Returns
    /// * `Result<(), EventSystemError>` - A Result indicating whether the operation was successful or not
    /// # Errors
    /// * `EventSystemError` - If there is an error during the operation
    pub async fn run_l2(&mut self) -> EventSystemResult<()> {
        futures::future::try_join_all(Self::get_l2_queues().into_iter().map(|queue_type| async move {
            let mut handler = self.create_event_handler(&queue_type).await.expect("Error while creating event handler");
            handler.run().await.expect("Error while running event handler");
        }))
        .await?;
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
