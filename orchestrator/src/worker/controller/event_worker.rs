use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{event::EventSystemResult, ConsumptionError};
use crate::types::priority_slot::{
    take_from_processing_slot_if_matches, take_from_verification_slot_if_matches, PriorityJobSlot,
};
use crate::types::queue::JobAction;
use crate::types::queue::{JobState, QueueType};
use crate::types::queue_control::{QueueControlConfig, QUEUES};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use color_eyre::eyre::eyre;
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn, Instrument, Span};
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

const QUEUE_GET_MESSAGE_WAIT_TIMEOUT_SECS: Duration = Duration::from_secs(30);
const QUEUE_NO_MESSAGE_SLEEP_DURATION: Duration = Duration::from_millis(1000);

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
        let queue_config = QUEUES.get(&queue_type).ok_or(ConsumptionError::QueueNotFound(queue_type.to_string()))?;
        let queue_control = queue_config.queue_control.clone();
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
    /// This function blocks until a compatible message is available (with a timeout)
    /// or an error occurs
    /// Messages with incompatible versions are re-enqueued for other orchestrator instances
    /// It returns a Result<Option<Delivery>, EventSystemError>
    pub async fn get_message(&self) -> EventSystemResult<Option<Delivery>> {
        let start = Instant::now();

        loop {
            match self.config.queue().consume_message_from_queue(self.queue_type.clone()).await {
                Ok(delivery) => return Ok(Some(delivery)),
                Err(crate::core::client::queue::QueueError::ErrorFromQueueError(omniqueue::QueueError::NoData)) => {
                    if start.elapsed() > QUEUE_GET_MESSAGE_WAIT_TIMEOUT_SECS {
                        return Ok(None);
                    }
                    tokio::time::sleep(QUEUE_NO_MESSAGE_SLEEP_DURATION).await;
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
        let worker_handler =
            JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_message.worker.clone());
        worker_handler
            .run_worker_if_enabled(self.config.clone())
            .await
            .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
        Ok(())
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
        JobHandlerService::handle_job_failure(queue_message.id, self.config.clone())
            .await
            .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
        Ok(())
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
            self.post_processing(result, message, &parsed_message).await
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
        info!("Starting {:?} worker (pool_size={})", self.queue_type, max_concurrent_tasks);

        loop {
            // Check if shutdown was requested at the start of each loop iteration
            if self.is_shutdown_requested() {
                info!("Shutdown requested, stopping message processing");
                break;
            }

            tokio::select! {
                biased;

                // 1. Handle completed tasks (highest priority)
                Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                    Self::handle_task_result(result);

                    if tasks.len() > max_concurrent_tasks {
                        warn!("Backpressure activated - waiting for tasks to complete. Active: {}", tasks.len());
                    }
                }

                // 2. Handle shutdown signal
                _ = self.cancellation_token.cancelled() => {
                    info!("Shutdown signal received, breaking from main loop");
                    break;
                }

                // 3. Check priority slot - pattern only matches if there's a message
                // If check_priority_slot() returns None, this branch is skipped
                // and select proceeds to normal queue. This prevents starvation.
                Some(priority_slot) = self.check_priority_slot(), if tasks.len() < max_concurrent_tasks => {
                    let job_queue_msg = JobQueueMessage { id: priority_slot.message.id };
                    let parsed_message = ParsedMessage::JobQueue(Box::new(job_queue_msg));
                    let delivery = priority_slot.delivery;

                    let worker = self.clone();
                    tasks.spawn(async move {
                        let result = worker.handle_message(&parsed_message).await;
                        // ACK/NACK the priority delivery after processing
                        match &result {
                            Ok(_) => {
                                if let Err(e) = delivery.ack().await {
                                    error!("Failed to ACK priority message: {:?}", e);
                                }
                            }
                            Err(_) => {
                                if let Err(e) = delivery.nack().await {
                                    error!("Failed to NACK priority message: {:?}", e);
                                }
                            }
                        }
                        result
                    });
                    debug!("Spawned PRIORITY task from slot, active: {}", tasks.len());
                }

                // 4. Process normal queue messages
                message_result = self.get_message(), if tasks.len() < max_concurrent_tasks => {
                    match message_result {
                        Ok(message) => {
                            match message {
                                Some(message) => {
                                    if let Ok(parsed_message) = self.parse_message(&message) {
                                        match &parsed_message {
                                    ParsedMessage::WorkerTrigger(msg) => {
                                        tracing::debug!(
                                            queue = %self.queue_type,
                                            worker_type = ?msg.worker,
                                            "Received message from queue"
                                        );
                                    }
                                    ParsedMessage::JobQueue(msg) => {
                                        tracing::debug!(
                                            queue = %self.queue_type,
                                            job_id = %msg.id,
                                            "Received message from queue"
                                        );
                                    }
                                }
                                        let worker = self.clone();
                                        tasks.spawn(async move {
                                            worker.process_message(message, parsed_message).await
                                        });
                                    }
                                },
                                None => sleep(QUEUE_NO_MESSAGE_SLEEP_DURATION).await,
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

    /// Check if there's a priority job in the global slot that matches this worker.
    ///
    /// Only job processing and verification workers check the priority slot.
    /// System queues (WorkerTrigger, JobHandleFailure, PriorityQueues) do not.
    ///
    /// # Returns
    /// * `Some(PriorityJobSlot)` if a matching priority job was found and taken
    /// * `None` if no matching job or this worker shouldn't check the slot
    async fn check_priority_slot(&self) -> Option<PriorityJobSlot> {
        // Only job processing/verification workers check the slot
        if !should_check_priority_slot(&self.queue_type) {
            return None;
        }

        let target_type = self.queue_type.target_job_type()?;
        let target_action = self.queue_type.target_action()?;

        // Check the appropriate slot based on action type
        match target_action {
            JobAction::Process => take_from_processing_slot_if_matches(&target_type).await,
            JobAction::Verify => take_from_verification_slot_if_matches(&target_type).await,
        }
    }

    /// Handle the result of a task
    /// This function handles the result of a task
    /// It logs the result of the task
    /// # Arguments
    /// * `result` - The result of the task
    fn handle_task_result(result: Result<EventSystemResult<()>, tokio::task::JoinError>) {
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                error!("Task failed with application error: {:?}", e);
            }
            Err(e) => {
                error!("Task panicked or was cancelled: {:?}", e);
            }
        }
    }
}

/// Determines if a queue type should check the priority slot.
/// System queues (WorkerTrigger, JobHandleFailure, Priority queues) should not check.
fn should_check_priority_slot(queue_type: &QueueType) -> bool {
    !matches!(
        queue_type,
        QueueType::WorkerTrigger
            | QueueType::JobHandleFailure
            | QueueType::PriorityProcessingQueue
            | QueueType::PriorityVerificationQueue
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_queues_should_not_check_priority_slot() {
        // System queues should not check priority slot
        assert!(!should_check_priority_slot(&QueueType::WorkerTrigger));
        assert!(!should_check_priority_slot(&QueueType::JobHandleFailure));
        assert!(!should_check_priority_slot(&QueueType::PriorityProcessingQueue));
        assert!(!should_check_priority_slot(&QueueType::PriorityVerificationQueue));
    }

    #[test]
    fn test_job_queues_should_check_priority_slot() {
        // Job processing queues should check priority slot
        assert!(should_check_priority_slot(&QueueType::SnosJobProcessing));
        assert!(should_check_priority_slot(&QueueType::ProvingJobProcessing));

        // Job verification queues should check priority slot
        assert!(should_check_priority_slot(&QueueType::SnosJobVerification));
        assert!(should_check_priority_slot(&QueueType::ProvingJobVerification));
    }
}
