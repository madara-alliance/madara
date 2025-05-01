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
use omniqueue::backends::SqsConsumer;
use omniqueue::{Delivery, QueueError};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span};

pub enum MessageType {
    Message(Delivery),
    NoMessage,
}

pub struct EventWorker {
    queue_type: QueueType,
    config: Arc<Config>,
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
        info!("Kicking in the Worker to Monitor the Queue {:?}", queue_type);
        Self { queue_type, config }
    }

    async fn consumer(&self) -> EventSystemResult<SqsConsumer> {
        Ok(self.config.clone().queue().get_consumer(self.queue_type.clone()).await.expect("error"))
    }

    /// get_message - Get the next message from the queue
    /// This function returns the next message from the queue
    /// It returns a Result<MessageType, EventSystemError> indicating whether the operation was successful or not
    pub async fn get_message(&self) -> EventSystemResult<Option<Delivery>> {
        let mut consumer = self.consumer().await?;
        // if let Some(consumer) = consumer.as_mut() {
        debug!("Waiting for message from queue {:?}", self.queue_type);
        match consumer.receive().await {
            Ok(delivery) => Ok(Some(delivery)),
            Err(QueueError::NoData) => Ok(None),
            Err(e) => {
                Err(EventSystemError::from(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() }))
            }
        }
        // } else {
        //     Err(EventSystemError::from(ConsumptionError::FailedToConsumeFromQueue {
        //         error_msg: "Consumer not initialized".to_string(),
        //     }))
        // }
    }

    /// parse_message - Parse the message received from the queue
    /// This function parses the message based on its type
    /// It returns a Result<ParsedMessage, EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be parsed
    /// # Returns
    /// * `Result<ParsedMessage, EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be parsed
    fn parse_message(&self, message: &Delivery) -> EventSystemResult<ParsedMessage> {
        match self.queue_type {
            QueueType::WorkerTrigger => WorkerTriggerMessage::parse_message(message).map(ParsedMessage::WorkerTrigger),
            _ => JobQueueMessage::parse_message(message).map(ParsedMessage::JobQueue),
        }
    }

    /// handle_message - Handle the message received from the queue
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be handled
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_message(&self, message: ParsedMessage) -> EventSystemResult<()> {
        match self.queue_type {
            QueueType::WorkerTrigger => {
                info!("Received message to Handler: {:?}", message);
                if let ParsedMessage::WorkerTrigger(worker_mes) = message {
                    let span = info_span!("worker_trigger", q = %self.queue_type, worker_id = %worker_mes.worker);
                    let _guard = span.enter();
                    let worker_handler =
                        JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_mes.worker.clone());
                    worker_handler
                        .run_worker_if_enabled(self.config.clone())
                        .await
                        .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
                    Ok(())
                } else {
                    Err(EventSystemError::from(ConsumptionError::Other(OtherError::from(eyre!(
                        "Expected WorkerTrigger message"
                    )))))
                }
            }
            QueueType::JobHandleFailure => match message {
                ParsedMessage::JobQueue(queue_message) => {
                    let span = info_span!("job_handle_failure", q = %self.queue_type, worker_id = %queue_message.id);
                    let _guard = span.enter();
                    JobHandlerService::handle_job_failure(queue_message.id, self.config.clone())
                        .await
                        .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
                    Ok(())
                }
                _ => Err(EventSystemError::from(ConsumptionError::Other(OtherError::from(eyre!(
                    "Expected JobQueue message"
                ))))),
            },
            _ => {
                let job_state: JobState = JobState::try_from(self.queue_type.clone())?;
                info!("Received message: {:?}, state: {:?}", message, job_state);

                match message {
                    ParsedMessage::JobQueue(queue_message) => {
                        let span = info_span!("job_queue", q = %self.queue_type, worker_id = %queue_message.id);
                        let _guard = span.enter();
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
                    _ => Err(EventSystemError::from(ConsumptionError::Other(OtherError::from(eyre!(
                        "Expected JobQueue message"
                    ))))),
                }
            }
        }
    }

    /// post_processing - Post process the message after handling
    /// This function acknowledges or negatively acknowledges the message based on the result of the handling
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `result` - The result of the message handling
    /// * `message` - The message to be post processed
    /// * `parsed_message` - The parsed message
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be post processed
    /// # Notes
    /// * This function is responsible for acknowledging or negatively acknowledging the message based on the result of the handling
    async fn post_processing(
        &self,
        result: EventSystemResult<()>,
        message: Delivery,
        parsed_message: ParsedMessage,
    ) -> EventSystemResult<()> {
        if let Err(ref error) = result {
            let (_error_context, consumption_error) = match &parsed_message {
                ParsedMessage::WorkerTrigger(msg) => {
                    let worker = &msg.worker;
                    tracing::error!("Failed to handle worker trigger {worker:?}. Error: {error:?}");
                    (
                        format!("Worker {worker:?} handling failed: {error}"),
                        ConsumptionError::FailedToSpawnWorker {
                            worker_trigger_type: worker.clone(),
                            error_msg: error.to_string(),
                        },
                    )
                }
                ParsedMessage::JobQueue(msg) => {
                    let job_id = &msg.id;
                    tracing::error!("Failed to handle job {job_id:?}. Error: {error:?}");
                    (
                        format!("Job {job_id:?} handling failed: {error}"),
                        ConsumptionError::FailedToHandleJob { job_id: *job_id, error_msg: error.to_string() },
                    )
                }
            };

            // Negative acknowledgment of the message so it can be retried
            message.nack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;

            // TODO: Since we are using SNS, we need to send the error message to the DLQ in future
            // self.config.alerts().send_message(error_context).await?;

            // Return the specific error
            return Err(consumption_error.into());
        }

        // Only acknowledge if processing was successful
        message.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;

        Ok(())
    }

    async fn process_message(&self, message: Delivery, parsed_message: ParsedMessage) -> EventSystemResult<()> {
        let result = self.handle_message(parsed_message.clone()).await;
        match self.post_processing(result, message, parsed_message.clone()).await {
            Ok(_) => {
                info!("Message processed successfully");
            }
            Err(e) => {
                match parsed_message.clone() {
                    ParsedMessage::WorkerTrigger(msg) => {
                        error!("Failed to handle worker trigger {msg:?}. Error: {e:?}");
                    }
                    ParsedMessage::JobQueue(msg) => {
                        error!("Failed to handle job {msg:?}. Error: {e:?}");
                    }
                }
                let _ = eyre!("Failed to process message: {:?}", e);
                error!("Failed to process message: {:?}", e);
            }
        }
        Ok(())
    }

    /// run - Run the event worker, by closly monitoring the queue and processing messages
    /// This function starts the event worker and processes messages from the queue
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the event worker cannot be started
    /// # Notes
    /// * This function runs indefinitely, processing messages from the queue
    /// * It will sleep for a short duration if no messages are received to prevent a tight loop
    /// * It will also sleep for a longer duration if an error occurs to prevent a tight loop
    /// * It will log errors and messages for debugging purposes
    pub async fn run(&self) -> EventSystemResult<()> {
        loop {
            match self.get_message().await {
                Ok(Some(message)) => match self.parse_message(&message) {
                    Ok(parsed_message) => {
                        self.process_message(message, parsed_message).await?;
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
