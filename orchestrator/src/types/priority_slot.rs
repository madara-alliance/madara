//! Priority Job Slot Module
//!
//! This module provides global slot mechanisms for priority job messages.
//! There are two slots: one for processing and one for verification.
//! The Priority Queue Workers place validated messages into their respective slots,
//! and job workers check and take messages that match their job type.

use crate::types::jobs::types::JobType;
use omniqueue::Delivery;
use std::sync::LazyLock;
use std::time::Instant;
use uuid::Uuid;

/// Message data for priority job slot (no action needed - determined by slot type)
#[derive(Debug, Clone)]
pub struct PriorityJobMessage {
    pub id: Uuid,
    pub job_type: JobType,
}

/// Represents a priority job message with its delivery handle for ack/nack
/// and timestamp for staleness tracking.
pub struct PriorityJobSlot {
    pub message: PriorityJobMessage,
    pub delivery: Delivery,
    pub placed_at: Instant,
}

impl PriorityJobSlot {
    /// Create a new priority job slot with the current timestamp.
    pub fn new(id: Uuid, job_type: JobType, delivery: Delivery) -> Self {
        Self { message: PriorityJobMessage { id, job_type }, delivery, placed_at: Instant::now() }
    }

    /// Check if this slot's message matches the given job type.
    pub fn matches(&self, job_type: &JobType) -> bool {
        self.message.job_type == *job_type
    }

    /// Check if this slot has been occupied for longer than the given duration.
    pub fn is_stale(&self, max_age_secs: u64) -> bool {
        self.placed_at.elapsed().as_secs() > max_age_secs
    }
}

// =============================================================================
// Processing Slot
// =============================================================================

/// Thread-safe global slot for priority processing job messages.
pub static PRIORITY_PROCESSING_SLOT: LazyLock<tokio::sync::Mutex<Option<PriorityJobSlot>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Take a priority job from the processing slot if it matches the given job type.
pub async fn take_from_processing_slot_if_matches(job_type: &JobType) -> Option<PriorityJobSlot> {
    let mut slot = PRIORITY_PROCESSING_SLOT.lock().await;
    if let Some(ref priority_job) = *slot {
        if priority_job.matches(job_type) {
            return slot.take();
        }
    }
    None
}

/// Place a priority job in the processing slot.
///
/// # Returns
/// * `true` if the job was placed successfully (slot was empty)
/// * `false` if the slot was already occupied
pub async fn place_in_processing_slot(job: PriorityJobSlot) -> bool {
    let mut slot = PRIORITY_PROCESSING_SLOT.lock().await;
    if slot.is_none() {
        *slot = Some(job);
        true
    } else {
        false
    }
}

/// Check if the processing slot is currently empty.
pub async fn is_processing_slot_empty() -> bool {
    PRIORITY_PROCESSING_SLOT.lock().await.is_none()
}

/// Clear the processing slot if the message is stale and return the delivery for NACKing.
///
/// # Returns
/// * `Some(Delivery)` if a stale message was cleared
/// * `None` if slot is empty or message is not stale
pub async fn clear_stale_from_processing_slot(max_age_secs: u64) -> Option<Delivery> {
    let mut slot = PRIORITY_PROCESSING_SLOT.lock().await;
    if let Some(ref priority_job) = *slot {
        if priority_job.is_stale(max_age_secs) {
            return slot.take().map(|s| s.delivery);
        }
    }
    None
}

// =============================================================================
// Verification Slot
// =============================================================================

/// Thread-safe global slot for priority verification job messages.
pub static PRIORITY_VERIFICATION_SLOT: LazyLock<tokio::sync::Mutex<Option<PriorityJobSlot>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(None));

/// Take a priority job from the verification slot if it matches the given job type.
pub async fn take_from_verification_slot_if_matches(job_type: &JobType) -> Option<PriorityJobSlot> {
    let mut slot = PRIORITY_VERIFICATION_SLOT.lock().await;
    if let Some(ref priority_job) = *slot {
        if priority_job.matches(job_type) {
            return slot.take();
        }
    }
    None
}

/// Place a priority job in the verification slot.
///
/// # Returns
/// * `true` if the job was placed successfully (slot was empty)
/// * `false` if the slot was already occupied
pub async fn place_in_verification_slot(job: PriorityJobSlot) -> bool {
    let mut slot = PRIORITY_VERIFICATION_SLOT.lock().await;
    if slot.is_none() {
        *slot = Some(job);
        true
    } else {
        false
    }
}

/// Check if the verification slot is currently empty.
pub async fn is_verification_slot_empty() -> bool {
    PRIORITY_VERIFICATION_SLOT.lock().await.is_none()
}

/// Clear the verification slot if the message is stale and return the delivery for NACKing.
///
/// # Returns
/// * `Some(Delivery)` if a stale message was cleared
/// * `None` if slot is empty or message is not stale
pub async fn clear_stale_from_verification_slot(max_age_secs: u64) -> Option<Delivery> {
    let mut slot = PRIORITY_VERIFICATION_SLOT.lock().await;
    if let Some(ref priority_job) = *slot {
        if priority_job.is_stale(max_age_secs) {
            return slot.take().map(|s| s.delivery);
        }
    }
    None
}
