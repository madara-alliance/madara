use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{event::EventSystemResult, ConsumptionError};
use crate::types::queue::{JobState, QueueType};
use crate::types::queue_control::{QueueControlConfig, QUEUES};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use color_eyre::eyre::eyre;
use omniqueue::backends::SqsConsumer;
use omniqueue::{Delivery, QueueError};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tracing::{debug, error, info, info_span};
use tokio::sync::Notify;
use std::sync::atomic::{AtomicBool, Ordering};

pub enum MessageType {
    Message(Delivery),
    NoMessage,
}

#[derive(Clone)]
pub struct EventWorker {
    config: Arc<Config>,
    queue_type: QueueType,
    queue_control: QueueControlConfig,
    shutdown_notify: Arc<Notify>,
    is_shutdown: Arc<AtomicBool>,
}

impl EventWorker {
    /// new - Create a new EventWorker
    /// This function creates a new EventWorker with the given configuration
    /// It returns a new EventWorker instance
    /// # Arguments
    /// * `config` - The configuration for the EventWorker
    /// # Returns
    /// * `EventSystemResult<EventWorker>` - A Result indicating whether the operation was successful or not
    pub fn new(queue_type: QueueType, config: Arc<Config>) -> EventSystemResult<Self> {
        info!("Kicking in the Worker to Queue {:?}", queue_type);
        let queue_config = QUEUES.get(&queue_type).ok_or(ConsumptionError::QueueNotFound(queue_type.to_string()))?;
        let queue_control = queue_config.queue_control.clone().unwrap_or_default();
        Ok(Self {
            queue_type,
            config,
            queue_control,
            shutdown_notify: Arc::new(Notify::new()),
            is_shutdown: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Triggers a graceful shutdown
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        self.is_shutdown.store(true, Ordering::SeqCst);
        self.shutdown_notify.notify_waiters();
        Ok(())
    }

    async fn consumer(&self) -> EventSystemResult<SqsConsumer> {
        Ok(self.config.clone().queue().get_consumer(self.queue_type.clone()).await.expect("error"))
    }

    /// get_message - Get the next message from the queue
    /// This function returns the next message from the queue
    /// It returns a Result<Option<Delivery>, EventSystemError> indicating whether the operation was successful or not
    pub async fn get_message(&self) -> EventSystemResult<Option<Delivery>> {
        let mut consumer = self.consumer().await?;
        debug!("Waiting for message from queue {:?}", self.queue_type);
        match consumer.receive().await {
            Ok(delivery) => Ok(Some(delivery)),
            Err(QueueError::NoData) => Ok(None),
            Err(e) => Err(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() })?,
        }
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

    /// handle_worker_trigger - Handle the message received from the queue for WorkerTrigger type
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be handled
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_worker_trigger(&self, message: &ParsedMessage) -> EventSystemResult<()> {
        if let ParsedMessage::WorkerTrigger(worker_mes) = message {
            let span = info_span!("worker_trigger", q = %self.queue_type, id = %worker_mes.worker);
            let _guard = span.enter();
            let worker_handler =
                JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_mes.worker.clone());
            worker_handler
                .run_worker_if_enabled(self.config.clone())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
            Ok(())
        } else {
            Err(ConsumptionError::Other(OtherError::from(eyre!("Expected WorkerTrigger message"))))?
        }
    }

    /// handle_job_failure - Handle the message received from the queue for JobHandleFailure type
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be handled
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_job_failure(&self, message: &ParsedMessage) -> EventSystemResult<()> {
        if let ParsedMessage::JobQueue(queue_message) = message {
            let span = info_span!("job_handle_failure", q = %self.queue_type, id = %queue_message.id);
            let _guard = span.enter();
            JobHandlerService::handle_job_failure(queue_message.id, self.config.clone())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
            Ok(())
        } else {
            Err(ConsumptionError::Other(OtherError::from(eyre!("Expected JobQueue message"))))?
        }
    }

    /// handle_job_queue - Handle the message received from the queue for JobQueue type
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be handled
    /// * `job_state` - The state of the job
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_job_queue(&self, message: &ParsedMessage, job_state: JobState) -> EventSystemResult<()> {
        info!("Received message: {:?}, state: {:?}", message, job_state);
        if let ParsedMessage::JobQueue(queue_message) = message {
            let span = info_span!("job_queue", q = %self.queue_type, id = %queue_message.id);
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
        } else {
            Err(ConsumptionError::Other(OtherError::from(eyre!("Expected JobQueue message"))))?
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
    async fn handle_message(&self, message: &ParsedMessage) -> EventSystemResult<()> {
        match self.queue_type {
            QueueType::WorkerTrigger => self.handle_worker_trigger(message).await,
            QueueType::JobHandleFailure => self.handle_job_failure(message).await,
            _ => {
                let job_state: JobState = JobState::try_from(self.queue_type.clone())?;
                self.handle_job_queue(message, job_state).await
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
        parsed_message: &ParsedMessage,
    ) -> EventSystemResult<()> {
        if let Err(ref error) = result {
            let (_error_context, consumption_error) = match parsed_message {
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

            message.nack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;

            // TODO: Since we are using SNS, we need to send the error message to the DLQ in future
            // self.config.alerts().send_message(error_context).await?;

            return Err(consumption_error.into());
        }

        message.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;
        Ok(())
    }

    /// process_message - Process the message received from the queue
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `message` - The message to be processed
    /// * `parsed_message` - The parsed message
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be processed
    /// # Notes
    /// * This function processes the message based on its type
    /// * It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// * It returns an EventSystemError if the message cannot be processed
    async fn process_message(&self, message: Delivery, parsed_message: ParsedMessage) -> EventSystemResult<()> {
        let result = self.handle_message(&parsed_message).await;
        match self.post_processing(result, message, &parsed_message).await {
            Ok(_) => {
                info!("Message processed successfully");
                Ok(())
            }
            Err(e) => {
                match parsed_message {
                    ParsedMessage::WorkerTrigger(msg) => {
                        error!("Failed to handle worker trigger {msg:?}. Error: {e:?}");
                    }
                    ParsedMessage::JobQueue(msg) => {
                        error!("Failed to handle job {msg:?}. Error: {e:?}");
                    }
                }
                let _ = eyre!("Failed to process message: {:?}", e);
                error!("Failed to process message: {:?}", e);
                Err(e)
            }
        }
    }

    /// run - Run the event worker, by closely monitoring the queue and processing messages
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
        let mut tasks = JoinSet::new();
        let max_concurrent_tasks = self.queue_control.max_message_count;
        info!("Starting worker with thread pool size: {}", max_concurrent_tasks);

        loop {
            // Check for shutdown signal
            if self.is_shutdown.load(Ordering::SeqCst) {
                info!("Shutdown signal received. Waiting for tasks to finish...");
                while let Some(res) = tasks.join_next().await {
                    if let Err(e) = res {
                        error!("Task failed: {:?}", e);
                    }
                }
                info!("All tasks finished. Exiting run loop.");
                break;
            }

            while tasks.len() >= max_concurrent_tasks {
                if let Some(Err(e)) = tasks.join_next().await {
                    error!("Task failed: {:?}", e);
                }
            }

            tokio::select! {
                // The biased keyword makes tokio::select! check branches in order,
                // prioritizing the shutdown signal check before message processing
                biased;
                _ = self.shutdown_notify.notified() => {
                    info!("Shutdown notify received. Exiting run loop soon.");
                    continue;
                }
                result = self.get_message() => {
                    match result {
                        Ok(Some(message)) => match self.parse_message(&message) {
                            Ok(parsed_message) => {
                                let worker = self.clone();
                                tasks.spawn(async move { worker.process_message(message, parsed_message).await });
                            }
                            Err(e) => {
                                error!("Failed to parse message: {:?}", e);
                            }
                        },
                        Ok(None) => {
                            sleep(Duration::from_secs(1)).await;
                        }
                        Err(e) => {
                            error!("Error receiving message: {:?}", e);
                            sleep(Duration::from_secs(5)).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
