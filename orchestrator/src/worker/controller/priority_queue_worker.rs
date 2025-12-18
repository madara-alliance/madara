//! Priority Queue Worker Module
//!
//! This module provides dedicated workers for processing priority job queues.
//! There are two workers: one for processing and one for verification.
//! Each worker reads messages from its priority queue, validates job status, and
//! places valid messages into a global slot for job workers to consume.

use crate::core::client::queue::QueueError;
use crate::core::config::Config;
use crate::error::event::EventSystemResult;
use crate::error::ConsumptionError;
use crate::types::jobs::types::JobStatus;
use crate::types::priority_slot::{
    clear_stale_from_processing_slot, clear_stale_from_verification_slot, is_processing_slot_empty,
    is_verification_slot_empty, place_in_processing_slot, place_in_verification_slot, PriorityJobSlot,
};
use crate::types::queue::QueueType;
use crate::types::queue_control::{
    PRIORITY_SLOT_CHECK_INTERVAL_MS, PRIORITY_SLOT_STALENESS_TIMEOUT_SECS, PRIORITY_SLOT_WAIT_TIMEOUT_SECS,
};
use crate::worker::parser::job_queue_message::JobQueueMessage;
use crate::worker::traits::message::MessageParser;
use omniqueue::Delivery;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// The action type this priority queue worker handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityAction {
    Processing,
    Verification,
}

impl std::fmt::Display for PriorityAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriorityAction::Processing => write!(f, "Processing"),
            PriorityAction::Verification => write!(f, "Verification"),
        }
    }
}

/// Dedicated worker for processing priority job queues.
///
/// This worker is responsible for:
/// 1. Waiting for the priority slot to be empty (with staleness check)
/// 2. Reading messages from the priority queue
/// 3. Validating job status before placing in slot
/// 4. ACKing messages that no longer need processing
/// 5. Placing valid messages into the global priority slot
#[derive(Clone)]
pub struct PriorityQueueWorker {
    config: Arc<Config>,
    action: PriorityAction,
    cancellation_token: CancellationToken,
}

impl PriorityQueueWorker {
    /// Create a new PriorityQueueWorker.
    ///
    /// # Arguments
    /// * `config` - The application configuration
    /// * `action` - The action type (Processing or Verification)
    /// * `cancellation_token` - Token for coordinated shutdown
    pub fn new(config: Arc<Config>, action: PriorityAction, cancellation_token: CancellationToken) -> Self {
        info!("Initializing Priority Queue Worker for {}", action);
        Self { config, action, cancellation_token }
    }

    /// Get the queue type this worker reads from.
    fn queue_type(&self) -> QueueType {
        match self.action {
            PriorityAction::Processing => QueueType::PriorityProcessingQueue,
            PriorityAction::Verification => QueueType::PriorityVerificationQueue,
        }
    }

    /// Check if shutdown has been requested.
    pub fn is_shutdown_requested(&self) -> bool {
        self.cancellation_token.is_cancelled()
    }

    /// Trigger a graceful shutdown.
    pub async fn shutdown(&self) -> EventSystemResult<()> {
        info!("Triggering shutdown for Priority Queue Worker ({})", self.action);
        self.cancellation_token.cancel();
        Ok(())
    }

    /// Run the priority queue worker.
    ///
    /// This runs an infinite loop that:
    /// 1. Waits for the slot to be empty (with staleness check)
    /// 2. Reads a message from the priority queue
    /// 3. Validates job status
    /// 4. Places valid messages into the global slot
    pub async fn run(&self) -> EventSystemResult<()> {
        info!("Starting Priority Queue Worker for {}", self.action);

        loop {
            if self.is_shutdown_requested() {
                info!("Priority Queue Worker ({}): Shutdown requested", self.action);
                break;
            }

            // Wait for slot to be empty (with staleness check)
            if !self.wait_for_slot_empty().await {
                // Timeout or shutdown, continue to check shutdown and retry
                continue;
            }

            // Try to get a message from the queue
            match self.get_message().await {
                Ok(Some(delivery)) => {
                    if let Err(e) = self.process_message(delivery).await {
                        error!("PQ Worker ({}): Error processing message: {:?}", self.action, e);
                        sleep(Duration::from_millis(100)).await;
                    }
                }
                Ok(None) => {
                    // No messages available, wait a bit before checking again
                    sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    error!("PQ Worker ({}): Error consuming from queue: {:?}", self.action, e);
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        info!("Priority Queue Worker ({}) shutdown complete", self.action);
        Ok(())
    }

    /// Wait for the priority slot to be empty.
    ///
    /// This method polls the slot status and handles staleness:
    /// - If a stale message is found, it NACKs the delivery and clears the slot
    /// - Returns true when the slot is empty
    /// - Returns false on timeout or shutdown
    async fn wait_for_slot_empty(&self) -> bool {
        let timeout = Duration::from_secs(*PRIORITY_SLOT_WAIT_TIMEOUT_SECS);
        let staleness_secs = *PRIORITY_SLOT_STALENESS_TIMEOUT_SECS;
        let check_interval = Duration::from_millis(*PRIORITY_SLOT_CHECK_INTERVAL_MS);
        let start = Instant::now();

        loop {
            if self.is_shutdown_requested() {
                return false;
            }

            if start.elapsed() > timeout {
                warn!("PQ Worker ({}): Timeout waiting for slot to empty after {}s", self.action, timeout.as_secs());
                return false;
            }

            // Check and clear stale messages
            if let Some(stale_delivery) = self.clear_stale_if_needed(staleness_secs).await {
                warn!("PQ Worker ({}): Cleared stale message from slot, NACKing", self.action);
                if let Err(e) = stale_delivery.nack().await {
                    error!("PQ Worker ({}): Failed to NACK stale delivery: {:?}", self.action, e);
                }
            }

            // Check if slot is empty
            if self.is_slot_empty().await {
                return true;
            }

            sleep(check_interval).await;
        }
    }

    /// Check if the slot (based on action type) is empty.
    async fn is_slot_empty(&self) -> bool {
        match self.action {
            PriorityAction::Processing => is_processing_slot_empty().await,
            PriorityAction::Verification => is_verification_slot_empty().await,
        }
    }

    /// Clear stale message from slot if needed.
    async fn clear_stale_if_needed(&self, max_age_secs: u64) -> Option<Delivery> {
        match self.action {
            PriorityAction::Processing => clear_stale_from_processing_slot(max_age_secs).await,
            PriorityAction::Verification => clear_stale_from_verification_slot(max_age_secs).await,
        }
    }

    /// Place a job in the appropriate slot.
    async fn place_in_slot(&self, job: PriorityJobSlot) -> bool {
        match self.action {
            PriorityAction::Processing => place_in_processing_slot(job).await,
            PriorityAction::Verification => place_in_verification_slot(job).await,
        }
    }

    /// Attempt to get a message from the priority queue.
    async fn get_message(&self) -> EventSystemResult<Option<Delivery>> {
        match self.config.queue().consume_message_from_queue(self.queue_type()).await {
            Ok(delivery) => Ok(Some(delivery)),
            Err(QueueError::ErrorFromQueueError(omniqueue::QueueError::NoData)) => Ok(None),
            Err(e) => {
                debug!("PQ Worker ({}): Queue check failed: {:?}", self.action, e);
                Ok(None)
            }
        }
    }

    /// Process a message from the priority queue.
    ///
    /// This method:
    /// 1. Parses the message (just job ID)
    /// 2. Fetches job from database to get job_type and status
    /// 3. Validates status and ACKs if action not needed
    /// 4. Places message in slot if valid
    async fn process_message(&self, delivery: Delivery) -> EventSystemResult<()> {
        // Parse the message (same format as JobQueueMessage - just id)
        let parsed_msg = match JobQueueMessage::parse_message(&delivery) {
            Ok(msg) => *msg,
            Err(e) => {
                error!("PQ Worker ({}): Failed to parse message: {:?}", self.action, e);
                // ACK malformed messages to prevent infinite retries
                if let Err(ack_err) = delivery.ack().await {
                    error!("PQ Worker ({}): Failed to ACK malformed message: {:?}", self.action, ack_err);
                }
                return Ok(());
            }
        };

        debug!("PQ Worker ({}): Processing job_id={}", self.action, parsed_msg.id);

        // Fetch job from database to check status and get job_type
        let job = match self.config.database().get_job_by_id(parsed_msg.id).await {
            Ok(Some(job)) => job,
            Ok(None) => {
                warn!("PQ Worker ({}): Job {} not found, ACKing message", self.action, parsed_msg.id);
                delivery.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;
                return Ok(());
            }
            Err(e) => {
                error!("PQ Worker ({}): Database error fetching job {}: {:?}", self.action, parsed_msg.id, e);
                // NACK to retry later
                delivery.nack().await.map_err(|e| ConsumptionError::FailedToNackMessage(e.0.to_string()))?;
                return Ok(());
            }
        };

        // Check if message should be ACKed (job no longer needs this action)
        if self.should_ack(&job.status) {
            debug!(
                "PQ Worker ({}): Job {} no longer needs action (status: {:?}), ACKing",
                self.action, parsed_msg.id, job.status
            );
            delivery.ack().await.map_err(|e| ConsumptionError::FailedToAcknowledgeMessage(e.0.to_string()))?;
            return Ok(());
        }

        // Create slot entry and place in slot
        let slot_job = PriorityJobSlot::new(parsed_msg.id, job.job_type, delivery);

        if self.place_in_slot(slot_job).await {
            debug!("PQ Worker ({}): Placed job {} in slot", self.action, parsed_msg.id);
        } else {
            // This shouldn't happen since we waited for slot to be empty,
            // but if there's a race condition, log it. The delivery is now
            // consumed by the failed placement attempt.
            error!("PQ Worker ({}): Race condition - failed to place job {} in slot", self.action, parsed_msg.id);
        }

        Ok(())
    }

    /// Determine if a message should be ACKed based on job status.
    ///
    /// ACK conditions:
    /// - Verification action AND status != PendingVerification
    /// - Processing action AND status NOT IN (PendingRetry, Created)
    fn should_ack(&self, status: &JobStatus) -> bool {
        match self.action {
            PriorityAction::Verification => !matches!(status, JobStatus::PendingVerification),
            PriorityAction::Processing => !matches!(status, JobStatus::Created | JobStatus::PendingRetry),
        }
    }
}
