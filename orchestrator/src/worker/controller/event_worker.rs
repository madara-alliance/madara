use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{event::EventSystemResult, ConsumptionError};
use crate::types::queue::{JobState, QueueType};
use crate::types::queue_control::{QueueControlConfig, QUEUES};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use color_eyre::eyre::eyre;
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, info_span, warn, Instrument, Span};
use uuid::Uuid;

pub enum MessageType {
    Message(Delivery),
    NoMessage,
}

#[derive(Clone)]
pub struct EventWorker {
    config: Arc<Config>,
    queue_type: QueueType,
    queue_control: QueueControlConfig,
    cancellation_token: CancellationToken,
}

impl EventWorker {
    /// new - Create a new EventWorker
    /// This function creates a new EventWorker with the given configuration
    /// It returns a new EventWorker instance
    /// # Arguments
    /// * `config` - The configuration for the EventWorker
    /// * `cancellation_token` - Token for coordinated shutdown
    /// # Returns
    /// * `EventSystemResult<EventWorker>` - A Result indicating whether the operation was successful or not
    pub fn new(
        queue_type: QueueType,
        config: Arc<Config>,
        cancellation_token: CancellationToken,
    ) -> EventSystemResult<Self> {
        info!("Initializing Event Worker for Queue {:?}", queue_type);

        let queue_config = QUEUES.get(&queue_type).ok_or(ConsumptionError::QueueNotFound(queue_type.to_string()))?;
        let mut queue_control = queue_config.queue_control.clone();

        // Override with service config values if provided via CLI/env args
        // This ensures CLI arguments like --max-concurrent-snos-jobs actually take effect
        let service_config = config.service_config();
let override_val = match queue_type {
    QueueType::SnosJobProcessing => service_config.max_concurrent_snos_jobs,
    QueueType::ProvingJobProcessing => service_config.max_concurrent_proving_jobs,
    _ => None,
};

if let Some(max_concurrent) = override_val {
    info!(
        "Overriding {:?} max_message_count from {} to {} (from service config)",
        queue_type, queue_control.max_message_count, max_concurrent
    );
    queue_control.max_message_count = max_concurrent;
}

        Ok(Self { queue_type, config, queue_control, cancellation_token })
    }

    /// Triggers a graceful shutdown
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        info!("Triggering shutdown for {} worker", self.queue_type);
        self.cancellation_token.cancel();
        Ok(())
    }

    /// Check if shutdown has been requested (non-blocking)
    pub fn is_shutdown_requested(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    /// Create a single span for the entire job processing
    fn create_job_span(&self, parsed_message: &ParsedMessage) -> Span {
        let correlation_id = Uuid::new_v4();
        match parsed_message {
            ParsedMessage::JobQueue(msg) => {
                tracing::info_span!(
                    "job_processing",
                    job_id = %msg.id,
                    subject_id = %msg.id,
                    queue = %self.queue_type,
                    job_type = %self.get_job_type_from_queue(),
                    correlation_id = %correlation_id,
                    trace_id = %correlation_id,
                    span_type = "Job",
                    external_id = tracing::field::Empty,
                    batch_id = tracing::field::Empty,
                    bucket_id = tracing::field::Empty,
                    block_start = tracing::field::Empty,
                    block_end = tracing::field::Empty,
                    aggregator_query_id = tracing::field::Empty
                )
            }
            ParsedMessage::WorkerTrigger(msg) => {
                tracing::info_span!(
                    "worker_trigger",
                    worker = ?msg.worker,
                    queue = %self.queue_type,
                    correlation_id = %correlation_id,
                    trace_id = %correlation_id,
                    span_type = "Worker"
                )
            }
        }
    }

    fn get_job_type_from_queue(&self) -> &str {
        match self.queue_type {
            QueueType::SnosJobProcessing | QueueType::SnosJobVerification => "SnosRun",
            QueueType::ProvingJobProcessing | QueueType::ProvingJobVerification => "ProofCreation",
            QueueType::ProofRegistrationJobProcessing | QueueType::ProofRegistrationJobVerification => {
                "ProofRegistration"
            }
            QueueType::DataSubmissionJobProcessing | QueueType::DataSubmissionJobVerification => "DataSubmission",
            QueueType::UpdateStateJobProcessing | QueueType::UpdateStateJobVerification => "StateTransition",
            QueueType::AggregatorJobProcessing | QueueType::AggregatorJobVerification => "Aggregator",
            _ => "Unknown",
        }
    }

    /// get_message - Get the next message from the queue with version-based filtering
    /// This function blocks until a compatible message is available or an error occurs
    /// Messages with incompatible versions are re-enqueued for other orchestrator instances
    /// It returns a Result<Delivery, EventSystemError> - never returns None
    pub async fn get_message(&self) -> EventSystemResult<Delivery> {
        loop {
            match self.config.clone().queue().consume_message_from_queue(self.queue_type.clone()).await {
                Ok(delivery) => return Ok(delivery),
                Err(crate::core::client::queue::QueueError::ErrorFromQueueError(omniqueue::QueueError::NoData)) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(e) => {
                    error!(
                        queue = ?self.queue_type,
                        error = %e,
                        "Failed to consume message from queue"
                    );
                    return Err(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() })?;
                }
            }
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
    /// * `worker_message` - The WorkerTrigger message to be handled
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_worker_trigger(&self, worker_message: &WorkerTriggerMessage) -> EventSystemResult<()> {
        let span = info_span!("worker_trigger", q = %self.queue_type, id = %worker_message.worker);

        async move {
            let worker_handler =
                JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_message.worker.clone());
            worker_handler
                .run_worker_if_enabled(self.config.clone())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
            Ok(())
        }
        .instrument(span)
        .await
    }

    /// handle_job_failure - Handle the message received from the queue for JobHandleFailure type
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `queue_message` - The JobQueue message to be handled
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_job_failure(&self, queue_message: &JobQueueMessage) -> EventSystemResult<()> {
        let span = info_span!("job_handle_failure", q = %self.queue_type, id = %queue_message.id);

        async move {
            JobHandlerService::handle_job_failure(queue_message.id, self.config.clone())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
            Ok(())
        }
        .instrument(span)
        .await
    }

    /// handle_job_queue - Handle the message received from the queue for JobQueue type
    /// This function processes the message based on its type
    /// It returns a Result<(), EventSystemError> indicating whether the operation was successful or not
    /// # Arguments
    /// * `queue_message` - The JobQueue message to be handled
    /// * `job_state` - The state of the job
    /// # Returns
    /// * `Result<(), EventSystemError>` - A result indicating whether the operation was successful or not
    /// # Errors
    /// * Returns an EventSystemError if the message cannot be handled
    async fn handle_job_queue(&self, queue_message: &JobQueueMessage, job_state: JobState) -> EventSystemResult<()> {
        debug!("Received message: {:?}, state: {:?}", queue_message, job_state);
        let span = info_span!("job_queue", q = %self.queue_type, id = %queue_message.id);

        async move {
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
        .instrument(span)
        .await
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
            QueueType::WorkerTrigger => {
                if let ParsedMessage::WorkerTrigger(worker_message) = message {
                    self.handle_worker_trigger(worker_message).await
                } else {
                    Err(ConsumptionError::Other(OtherError::from(eyre!("Expected WorkerTrigger message"))))?
                }
            }
            QueueType::JobHandleFailure => {
                if let ParsedMessage::JobQueue(queue_message) = message {
                    self.handle_job_failure(queue_message).await
                } else {
                    Err(ConsumptionError::Other(OtherError::from(eyre!("Expected JobQueue message"))))?
                }
            }
            _ => {
                let job_state: JobState = JobState::try_from(self.queue_type.clone())?;
                if let ParsedMessage::JobQueue(queue_message) = message {
                    self.handle_job_queue(queue_message, job_state).await
                } else {
                    Err(ConsumptionError::Other(OtherError::from(eyre!("Expected JobQueue message"))))?
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
        let span = self.create_job_span(&parsed_message);
        async move {
            let result = self.handle_message(&parsed_message).await;
            match self.post_processing(result, message, &parsed_message).await {
                Ok(_) => {
                    debug!("Message processed successfully");
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
                    // Error already logged above, no additional action needed
                    error!("Failed to process message: {:?}", e);
                    Err(e)
                }
            }
        }
        .instrument(span)
        .await
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
            // Check if shutdown was requested at the start of each loop iteration
            if self.is_shutdown_requested() {
                info!("Shutdown requested, stopping message processing");
                break;
            }

            tokio::select! {
                biased;
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    Self::handle_task_result(result);

                    if tasks.len() > max_concurrent_tasks {
                        warn!("Backpressure activated - waiting for tasks to complete. Active: {}", tasks.len());
                    } else {
                        debug!("Task completed, active tasks: {}", tasks.len());
                    }
                }

                // Handle shutdown signal
                _ = self.cancellation_token.cancelled() => {
                    info!("Shutdown signal received, breaking from main loop");
                    break;
                }

                // Process new messages (with backpressure)
                message_result = self.get_message(), if tasks.len() < max_concurrent_tasks => {
                    match message_result {
                        Ok(message) => {
                            if let Ok(parsed_message) = self.parse_message(&message) {
                                let worker = self.clone();
                                tasks.spawn(async move {
                                    worker.process_message(message, parsed_message).await
                                });
                                debug!("Spawned task, active: {}", tasks.len());
                            }
                        }
                        Err(e) => {
                            error!("Error receiving message: {:?}", e);
                            sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }

        // Wait for remaining tasks to complete during shutdown
        info!("Waiting for {} remaining tasks to complete", tasks.len());
        while let Some(result) = tasks.join_next().await {
            Self::handle_task_result(result);
        }
        info!("All tasks completed, worker shutdown complete");

        Ok(())
    }

    /// Handle the result of a task
    /// This function handles the result of a task
    /// It logs the result of the task
    /// # Arguments
    /// * `result` - The result of the task
    fn handle_task_result(result: Result<EventSystemResult<()>, tokio::task::JoinError>) {
        match result {
            Ok(Ok(_)) => {
                debug!("Task completed successfully");
            }
            Ok(Err(e)) => {
                error!("Task failed with application error: {:?}", e);
            }
            Err(e) => {
                error!("Task panicked or was cancelled: {:?}", e);
            }
        }
    }
}
