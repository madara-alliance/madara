use crate::core::config::Config;
use crate::error::event::EventSystemResult;
use crate::error::ConsumptionError;
use crate::types::queue::QueueType;
use crate::types::worker::{MessageType, WorkerConfig};
use crate::worker::parser::job_queue_message::JobQueueMessage;
use crate::worker::traits::message::MessageParser;
use crate::OrchestratorResult;
use omniqueue::backends::{SqsConsumer, SqsProducer};
use omniqueue::{Delivery, QueueError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

#[derive(Clone)]
pub struct EventWorker {
    queue_type: QueueType,
    worker_config: Arc<WorkerConfig>,
    config: Arc<Config>,
    consumer: Option<Arc<SqsConsumer>>,
    producer: Option<Arc<SqsProducer>>,
}

impl EventWorker {
    /// new - Create a new EventWorker
    /// This function creates a new EventWorker with the given configuration
    /// It returns a new EventWorker instance
    /// # Arguments
    /// * `config` - The configuration for the EventWorker
    /// # Returns
    /// * `EventWorker` - A new EventWorker instance
    pub fn new(queue_type: QueueType, worker_config: Arc<WorkerConfig>, config: Arc<Config>) -> Self {
        Self { queue_type, worker_config, config, consumer: None, producer: None }
    }

    /// consumer - returns a consumer for the queue type
    /// if the consumer is not initialized, it will be initialized
    /// and returned
    async fn consumer(&mut self) -> EventSystemResult<Arc<SqsConsumer>> {
        if self.consumer.is_none() {
            self.consumer = Some(Arc::new(self.config.queue().get_consumer(self.queue_type.clone()).await?));
        }
        Ok(self.consumer.clone().unwrap())
    }

    /// producer - returns a producer for the queue type
    /// if the producer is not initialized, it will be initialized
    /// and returned
    async fn producer(&mut self) -> EventSystemResult<Arc<SqsProducer>> {
        if self.producer.is_none() {
            self.producer = Some(Arc::new(self.config.queue().get_producer(self.queue_type.clone()).await?));
        }
        Ok(self.producer.clone().unwrap())
    }
    /// get_message - Get the next message from the queue
    /// This function returns the next message from the queue
    /// It returns a Result<MessageType, EventSystemError> indicating whether the operation was successful or not
    async fn get_message(&mut self) -> EventSystemResult<MessageType> {
        let consumer = self.consumer().await?;
        match consumer.receive().await {
            Ok(delivery) => {
                tracing::debug!(queue = %self.queue_type, "Message received from queue");
                Ok(MessageType::Message(delivery))
            }
            Err(QueueError::NoData) => {
                tracing::debug!(queue = %self.queue_type, "No message in queue");
                Ok(MessageType::NoMessage)
            }
            Err(e) => Err(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() })?,
        }
    }

    fn parse_message(message: &Delivery) -> EventSystemResult<Box<JobQueueMessage>> {
        JobQueueMessage::parse_message(message)
    }

    async fn handle_message(&self, message: Delivery) -> EventSystemResult<()> {
        Ok(())
    }

    async fn process_message(&mut self) -> EventSystemResult<()> {
        let message = match self.get_message().await? {
            MessageType::Message(delivery) => {
                tracing::debug!(queue = %self.queue_type, "Message received from queue");
                delivery
            }
            MessageType::NoMessage => {
                tracing::debug!(queue = %self.queue_type, "No message in queue");
                return Ok(());
            }
        };
        let job_message = Self::parse_message(&message)?;
        tracing::info!(queue = %self.queue_type, job_id = %job_message.id, "Processing job message");

        // if let Some(job_message) = job_message {
        //     tracing::info!(queue = %self.queue_type, job_id = %job_message.id, "Processing job message");
        //     /// Handle the Job here
        //     /// TODO: Handle the error Here so that we can handle the error and retry
        // } else {
        //     tracing::warn!(queue = %self.queue_type, "Received empty job message");
        // }
        Ok(())
    }

    pub async fn run(&mut self) -> EventSystemResult<()> {
        loop {
            /// TODO: Handle the error Here so that we can handle the error and retry
            self.process_message().await.expect("Error while processing message");
            /// CLARIFICATION: why do we need to sleep here?
            /// since we might need this in the other language to release memory
            sleep(Duration::from_millis(500)).await;
        }
    }
}
