use crate::core::config::Config;
use crate::error::{
    event::{EventSystemError, EventSystemResult},
    ConsumptionError,
};
use crate::types::queue::QueueType;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use omniqueue::backends::{SqsConsumer, SqsProducer};
use omniqueue::{Delivery, QueueError};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;
use tracing::{error, info};

pub enum MessageType {
    Message(Delivery),
    NoMessage,
}

pub struct EventWorker {
    queue_type: QueueType,
    config: Arc<Config>,
    consumer: Arc<Mutex<Option<SqsConsumer>>>,
    producer: Arc<Mutex<Option<SqsProducer>>>,
}

impl EventWorker {
    /// new - Create a new EventWorker
    /// This function creates a new EventWorker with the given configuration
    /// It returns a new EventWorker instance
    /// # Arguments
    /// * `config` - The configuration for the EventWorker
    /// # Returns
    /// * `EventWorker` - A new EventWorker instance
    pub fn new(queue_type: QueueType, config: Arc<Config>) -> Self {
        Self { queue_type, config, consumer: Arc::new(Mutex::new(None)), producer: Arc::new(Mutex::new(None)) }
    }

    /// consumer - returns a consumer for the queue type
    /// if the consumer is not initialized, it will be initialized
    /// and returned
    pub async fn consumer(&self) -> EventSystemResult<Arc<Mutex<Option<SqsConsumer>>>> {
        Ok(self.consumer.clone())
    }

    /// producer - returns a producer for the queue type
    /// if the producer is not initialized, it will be initialized
    /// and returned
    pub async fn producer(&self) -> EventSystemResult<Arc<Mutex<Option<SqsProducer>>>> {
        Ok(self.producer.clone())
    }

    /// get_message - Get the next message from the queue
    /// This function returns the next message from the queue
    /// It returns a Result<MessageType, EventSystemError> indicating whether the operation was successful or not
    pub async fn get_message(&self) -> EventSystemResult<Option<Delivery>> {
        let mut consumer = self.consumer.lock().await;
        if let Some(consumer) = consumer.as_mut() {
            match consumer.receive().await {
                Ok(delivery) => Ok(Some(delivery)),
                Err(QueueError::NoData) => Ok(None),
                Err(e) => {
                    Err(EventSystemError::from(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() }))
                }
            }
        } else {
            Err(EventSystemError::from(ConsumptionError::FailedToConsumeFromQueue {
                error_msg: "Consumer not initialized".to_string(),
            }))
        }
    }

    fn parse_message(&self, message: &Delivery) -> EventSystemResult<ParsedMessage> {
        match self.queue_type {
            QueueType::WorkerTrigger => WorkerTriggerMessage::parse_message(message).map(ParsedMessage::WorkerTrigger),
            _ => JobQueueMessage::parse_message(message).map(ParsedMessage::JobQueue),
        }
    }

    async fn handle_message(&self, message: ParsedMessage) -> EventSystemResult<()> {
        // For now, just log the message
        info!(queue = %self.queue_type, "Handling message");
        Ok(())
    }

    pub async fn run(&self) -> EventSystemResult<()> {
        loop {
            match self.get_message().await {
                Ok(Some(message)) => match self.parse_message(&message) {
                    Ok(parsed_message) => {
                        if let Err(e) = self.handle_message(parsed_message).await {
                            error!("Failed to handle message: {:?}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse message: {:?}", e);
                    }
                },
                Ok(None) => {
                    // Sleep to prevent tight loop and allow memory cleanup
                    sleep(Duration::from_secs(1)).await;
                }
                Err(e) => {
                    error!("Error receiving message: {:?}", e);
                    // Sleep before retrying to prevent tight loop
                    sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }
}
