use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{
    event::{EventSystemError, EventSystemResult},
    ConsumptionError,
};
use crate::types::queue::{JobState, QueueType};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use color_eyre::eyre::eyre;
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
        match self.queue_type {
            QueueType::WorkerTrigger => {
                if let ParsedMessage::WorkerTrigger(_) = message {
                    Ok(())
                } else {
                    Err(EventSystemError::from(ConsumptionError::Other(
                        OtherError::from(eyre!("Expected WorkerTrigger message"))
                    )))
                }
            }
            QueueType::JobHandleFailure => {
                match message {
                    ParsedMessage::JobQueue(queue_message) => {
                        JobHandlerService::handle_job_failure(queue_message.id, self.config.clone())
                            .await
                            .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
                        Ok(())
                    }
                    _ => Err(EventSystemError::from(ConsumptionError::Other(
                        OtherError::from(eyre!("Expected JobQueue message"))
                    )))
                }
            }
            _ => {
                let job_state: JobState = JobState::try_from(self.queue_type.clone())?;
                info!("Received message: {:?}, state: {:?}", message, job_state);

                match message {
                    ParsedMessage::JobQueue(queue_message) => {
                        match job_state {
                            JobState::Processing => {
                                JobHandlerService::process_job(queue_message.id, self.config.clone())
                                    .await
                                    .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
                            }
                            JobState::Verification => {
                                JobHandlerService::verify_job(queue_message.id, self.config.clone())
                                    .await
                                    .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
                            }
                        }
                        Ok(())
                    }
                    _ => Err(EventSystemError::from(ConsumptionError::Other(
                        OtherError::from(eyre!("Expected JobQueue message"))
                    )))
                }
            }
        }
    }

    async fn post_processing(&self, result: EventSystemResult<()>, message: Delivery, parsed_message: ParsedMessage) -> EventSystemResult<()> {
        if let Err(ref error) = result {
            // Log error with context
            let (error_context, consumption_error) = match &parsed_message {
                ParsedMessage::WorkerTrigger(msg) => {
                    let worker = &msg.worker;
                    tracing::error!("Failed to handle worker trigger {worker:?}. Error: {error:?}");
                    (
                        format!("Worker {worker:?} handling failed: {error}"),
                        ConsumptionError::FailedToSpawnWorker {
                            worker_trigger_type: worker.clone(),
                            error_msg: error.to_string(),
                        }
                    )
                },
                ParsedMessage::JobQueue(msg) => {
                    let job_id = &msg.id;
                    tracing::error!("Failed to handle job {job_id:?}. Error: {error:?}");
                    (
                        format!("Job {job_id:?} handling failed: {error}"),
                        ConsumptionError::FailedToHandleJob {
                            job_id: job_id.clone(),
                            error_msg: error.to_string(),
                        }
                    )
                }
            };

            // Send alert about the error
            self.config
                .alerts()
                .send_message(error_context)
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;

            // Negative acknowledgment of the message so it can be retried
            message
                .nack()
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;

            // Return the specific error
            return Err(consumption_error.into());
        }

        // Only acknowledge if processing was successful
        message
            .ack()
            .await
            .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;

        Ok(())
    }

    pub async fn run(&self) -> EventSystemResult<()> {
        loop {
            match self.get_message().await {
                Ok(Some(message)) => match self.parse_message(&message) {
                    Ok(parsed_message) => {
                        let result = self.handle_message(parsed_message.clone()).await;
                        self.post_processing(result, message, parsed_message).await?;
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
