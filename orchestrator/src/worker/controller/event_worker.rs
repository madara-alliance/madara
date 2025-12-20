use crate::core::config::Config;
use crate::error::other::OtherError;
use crate::error::{event::EventSystemResult, ConsumptionError};
use crate::types::queue::QueueType;
use crate::types::queue_control::{QueueControlConfig, QUEUES};
use crate::worker::event_handler::service::JobHandlerService;
use crate::worker::parser::worker_trigger_message::WorkerTriggerMessage;
use crate::worker::traits::message::MessageParser;
use color_eyre::eyre::eyre;
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn, Instrument, Span};
use uuid::Uuid;

/// EventWorker handles WorkerTrigger queue consumption
/// NOTE: Job processing is now handled by greedy workers, not SQS
#[derive(Clone)]
pub struct EventWorker {
    config: Arc<Config>,
    queue_type: QueueType,
    queue_control: QueueControlConfig,
    cancellation_token: CancellationToken,
}

impl EventWorker {
    /// new - Create a new EventWorker (WorkerTrigger queue only)
    pub fn new(
        queue_type: QueueType,
        config: Arc<Config>,
        cancellation_token: CancellationToken,
    ) -> EventSystemResult<Self> {
        // Validate that we only create workers for WorkerTrigger
        if queue_type != QueueType::WorkerTrigger {
            return Err(ConsumptionError::Other(
                OtherError::from(eyre!(
                    "EventWorker only supports WorkerTrigger queue. Job processing uses greedy mode."
                ))
                .into(),
            ))?;
        }

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

    /// Create a span for worker trigger processing
    fn create_trigger_span(&self, worker_message: &WorkerTriggerMessage) -> Span {
        let correlation_id = Uuid::new_v4();
        tracing::info_span!(
            "worker_trigger",
            worker = ?worker_message.worker,
            queue = %self.queue_type,
            correlation_id = %correlation_id,
            trace_id = %correlation_id,
            span_type = "Worker"
        )
    }

    /// get_message - Get the next message from the WorkerTrigger queue
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

    /// parse_message - Parse WorkerTrigger message
    fn parse_message(&self, message: &Delivery) -> EventSystemResult<WorkerTriggerMessage> {
        WorkerTriggerMessage::parse_message(message).map(|boxed| *boxed)
    }

    /// handle_worker_trigger - Execute the worker trigger
    async fn handle_worker_trigger(&self, worker_message: &WorkerTriggerMessage) -> EventSystemResult<()> {
        let worker_handler =
            JobHandlerService::get_worker_handler_from_worker_trigger_type(worker_message.worker.clone());
        worker_handler
            .run_worker_if_enabled(self.config.clone())
            .await
            .map_err(|e| ConsumptionError::Other(OtherError::from(e.to_string())))?;
        Ok(())
    }

    /// post_processing - Acknowledge or nack the message
    async fn post_processing(
        &self,
        result: EventSystemResult<()>,
        message: Delivery,
        worker_message: &WorkerTriggerMessage,
    ) -> EventSystemResult<()> {
        if let Err(ref error) = result {
            let worker = &worker_message.worker;
            tracing::error!("Failed to handle worker trigger {worker:?}. Error: {error:?}");

            message.nack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;

            return Err(ConsumptionError::FailedToSpawnWorker {
                worker_trigger_type: worker.clone(),
                error_msg: error.to_string(),
            }
            .into());
        }

        message.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;
        Ok(())
    }

    /// process_message - Process a WorkerTrigger message
    async fn process_message(&self, message: Delivery, worker_message: WorkerTriggerMessage) -> EventSystemResult<()> {
        let span = self.create_trigger_span(&worker_message);
        async move {
            let result = self.handle_worker_trigger(&worker_message).await;
            self.post_processing(result, message, &worker_message).await
        }
        .instrument(span)
        .await
    }

    /// run - Run the WorkerTrigger event worker
    pub async fn run(&self) -> EventSystemResult<()> {
        let mut tasks = JoinSet::new();
        let max_concurrent_tasks = self.queue_control.max_message_count;
        info!("Starting {:?} worker (pool_size={})", self.queue_type, max_concurrent_tasks);

        loop {
            // Check if shutdown was requested
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
                            if let Ok(worker_message) = self.parse_message(&message) {
                                tracing::debug!(
                                    queue = %self.queue_type,
                                    worker_type = ?worker_message.worker,
                                    "Received message from queue"
                                );

                                let worker = self.clone();
                                tasks.spawn(async move {
                                    worker.process_message(message, worker_message).await
                                });
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
