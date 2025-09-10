use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{event::EventSystemResult, ConsumptionError};
use crate::types::queue::{JobState, QueueType};
use crate::types::queue_control::{QueueControlConfig, QUEUES};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::{job_queue_message::JobQueueMessage, worker_trigger_message::WorkerTriggerMessage};
use crate::worker::traits::message::{MessageParser, ParsedMessage};
use omniqueue::backends::SqsConsumer;
use omniqueue::{Delivery, QueueError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, Instrument, Span};
use uuid::Uuid;

#[derive(Clone)]
pub struct EventWorker {
    config: Arc<Config>,
    queue_type: QueueType,
    _queue_control: QueueControlConfig,
    cancellation_token: CancellationToken,
}

impl EventWorker {
    pub fn new(
        queue_type: QueueType,
        config: Arc<Config>,
        cancellation_token: CancellationToken,
    ) -> EventSystemResult<Self> {
        info!("Initializing Event Worker for Queue {:?}", queue_type);

        let queue_config = QUEUES.get(&queue_type).ok_or(ConsumptionError::QueueNotFound(queue_type.to_string()))?;

        let queue_control = queue_config.queue_control.clone();

        Ok(Self { queue_type, config, _queue_control: queue_control, cancellation_token })
    }

    /// Triggers a graceful shutdown
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        info!("Triggering shutdown for {} worker", self.queue_type);
        self.cancellation_token.cancel();
        Ok(())
    }

    /// Main run loop
    pub async fn run(&self) -> EventSystemResult<()> {
        info!(
            queue = %self.queue_type,
            "Starting Event Worker"
        );

        loop {
            if self.cancellation_token.is_cancelled() {
                info!(queue = %self.queue_type, "Shutdown signal received");
                break;
            }

            // Get message from queue
            let message = match self.get_message().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!(queue = %self.queue_type, error = %e, "Failed to get message");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Parse the message
            let parsed_message = match self.parse_message(&message) {
                Ok(msg) => msg,
                Err(e) => {
                    error!(error = %e, "Failed to parse message");
                    let _ = message.nack().await;
                    continue;
                }
            };

            // Process message with single root span
            let result = self.process_message(message, parsed_message).await;

            if let Err(e) = result {
                error!(error = %e, "Message processing failed");
            }
        }

        Ok(())
    }

    /// Process message with a single root span that captures all job processing
    async fn process_message(&self, message: Delivery, parsed_message: ParsedMessage) -> EventSystemResult<()> {
        let start = Instant::now();

        // Create single root span based on job ID or trigger type
        let span = self.create_job_span(&parsed_message);

        // Everything inside this async block will be automatically captured under the span
        async move {
            info!(
                queue = %self.queue_type,
                "Starting job processing"
            );

            // Handle the message - all logs inside will be captured
            let handling_result = match parsed_message {
                ParsedMessage::JobQueue(ref msg) => self.handle_job_queue(msg).await,
                ParsedMessage::WorkerTrigger(ref msg) => self.handle_worker_trigger(msg).await,
            };

            // Post-processing
            let ack_result = match &handling_result {
                Ok(_) => {
                    info!("Job completed successfully, acknowledging message");
                    message.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))
                }
                Err(e) => {
                    error!(error = %e, "Job failed, sending to DLQ");
                    message.nack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))
                }
            };

            let duration = start.elapsed();
            match &parsed_message {
                ParsedMessage::JobQueue(msg) => info!(
                    job_id = %msg.id,
                    duration_ms = duration.as_millis(),
                    success = handling_result.is_ok(),
                    "Job processing completed"
                ),
                ParsedMessage::WorkerTrigger(msg) => info!(
                    worker = ?msg.worker,
                    duration_ms = duration.as_millis(),
                    success = handling_result.is_ok(),
                    "Worker trigger completed"
                ),
            }

            handling_result.and(ack_result.map_err(|e| e.into()))
        }
        .instrument(span)
        .await
    }

    /// Create a single span for the entire job processing
    fn create_job_span(&self, parsed_message: &ParsedMessage) -> Span {
        match parsed_message {
            ParsedMessage::JobQueue(msg) => {
                // Use job ID as the primary identifier
                tracing::info_span!(
                    "job_processing",
                    job_id = %msg.id,
                    queue = %self.queue_type,
                    job_type = %self.get_job_type_from_queue(),
                    trace_id = %msg.id,  // Use job ID as trace ID for correlation
                    span_type = "root"
                )
            }
            ParsedMessage::WorkerTrigger(msg) => {
                // For triggers, create a unique ID
                let trigger_id = Uuid::new_v4();
                tracing::info_span!(
                    "worker_trigger",
                    trigger_id = %trigger_id,
                    worker = ?msg.worker,
                    queue = %self.queue_type,
                    trace_id = %trigger_id,
                    span_type = "root"
                )
            }
        }
    }

    /// Handle job queue message - all logs inside are captured by the span
    async fn handle_job_queue(&self, queue_message: &JobQueueMessage) -> EventSystemResult<()> {
        let job_state = JobState::try_from(self.queue_type.clone())?;

        info!(
            job_id = %queue_message.id,
            state = ?job_state,
            "Processing job message"
        );

        // Call the appropriate handler - all its logs will be captured
        match job_state {
            JobState::Processing => {
                debug!(job_id = %queue_message.id, "Calling JobHandlerService::process_job");

                JobHandlerService::process_job(queue_message.id, self.config.clone()).await.map_err(|e| {
                    error!(job_id = %queue_message.id, error = %e, "Job processing failed");
                    ConsumptionError::Other(OtherError::from(e.to_string()))
                })?;

                info!(job_id = %queue_message.id, "Job processing completed");
            }
            JobState::Verification => {
                debug!(job_id = %queue_message.id, "Calling JobHandlerService::verify_job");

                JobHandlerService::verify_job(queue_message.id, self.config.clone()).await.map_err(|e| {
                    error!(job_id = %queue_message.id, error = %e, "Job verification failed");
                    ConsumptionError::Other(OtherError::from(e.to_string()))
                })?;

                info!(job_id = %queue_message.id, "Job verification completed");
            }
        }

        Ok(())
    }

    /// Handle worker trigger message - all logs inside are captured by the span
    async fn handle_worker_trigger(&self, worker_message: &WorkerTriggerMessage) -> EventSystemResult<()> {
        info!(
            worker_type = ?worker_message.worker,
            "Processing worker trigger"
        );

        let worker_handler =
            JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_message.worker.clone());

        debug!(worker_type = ?worker_message.worker, "Running worker");

        worker_handler.run_worker_if_enabled(self.config.clone()).await.map_err(|e| {
            error!(worker_type = ?worker_message.worker, error = %e, "Worker trigger failed");
            ConsumptionError::Other(OtherError::from(e.to_string()))
        })?;

        info!(worker_type = ?worker_message.worker, "Worker trigger completed");

        Ok(())
    }

    // Helper methods

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

    async fn consumer(&self) -> EventSystemResult<SqsConsumer> {
        Ok(self.config.clone().queue().get_consumer(self.queue_type.clone()).await.expect("error"))
    }

    async fn get_message(&self) -> EventSystemResult<Delivery> {
        let mut consumer = self.consumer().await?;
        loop {
            debug!("Polling for message from queue {:?}", self.queue_type);
            match consumer.receive().await {
                Ok(delivery) => return Ok(delivery),
                Err(QueueError::NoData) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(e) => return Err(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() })?,
            }
        }
    }

    fn parse_message(&self, message: &Delivery) -> EventSystemResult<ParsedMessage> {
        match self.queue_type {
            QueueType::WorkerTrigger => WorkerTriggerMessage::parse_message(message).map(ParsedMessage::WorkerTrigger),
            _ => JobQueueMessage::parse_message(message).map(ParsedMessage::JobQueue),
        }
    }
}
