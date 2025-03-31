use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::error::Result;

/// Queue types for different job processing
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueType {
    /// Snos job processing queue
    SnosJobProcessing,

    /// Snos job verification queue
    SnosJobVerification,

    /// Proving job processing queue
    ProvingJobProcessing,

    /// Proving job verification queue
    ProvingJobVerification,

    /// Proof registration job processing queue
    ProofRegistrationJobProcessing,

    /// Proof registration job verification queue
    ProofRegistrationJobVerification,

    /// Data submission job processing queue
    DataSubmissionJobProcessing,

    /// Data submission job verification queue
    DataSubmissionJobVerification,

    /// Update state job processing queue
    UpdateStateJobProcessing,

    /// Update state job verification queue
    UpdateStateJobVerification,

    /// Job handle failure queue
    JobHandleFailure,

    /// Worker trigger queue
    WorkerTrigger,

    /// Custom queue
    Custom(String),
}

/// Job types for different processing tasks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobType {
    /// Snos job
    Snos,

    /// Proving job
    Proving,

    /// Proof registration job
    ProofRegistration,

    /// Data submission job
    DataSubmission,

    /// Update state job
    UpdateState,

    /// Custom job
    Custom(String),
}

/// Queue configuration
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Queue name
    pub name: QueueType,

    /// Visibility timeout in seconds
    pub visibility_timeout: i32,

    /// Dead letter queue name (if any)
    pub dead_letter_queue: Option<QueueType>,

    /// Maximum receive count before sending to DLQ
    pub max_receive_count: Option<i32>,
}

/// Message delivery from a queue
#[derive(Debug, Clone)]
pub struct Delivery {
    /// Message body
    pub body: String,

    /// Message ID
    pub message_id: String,

    /// Receipt handle for acknowledgment
    pub receipt_handle: String,

    /// Message attributes
    pub attributes: std::collections::HashMap<String, String>,
}

/// Job queue message
#[derive(Debug, Serialize, Deserialize)]
pub struct JobQueueMessage {
    /// Job ID
    pub id: Uuid,

    /// Job type
    pub job_type: Option<JobType>,

    /// Custom data
    pub custom_data: Option<serde_json::Value>,
}

/// Trait for mapping job types to queue names
pub trait QueueNameForJobType {
    /// Get the processing queue name for a job type
    fn process_queue_name(&self) -> QueueType;

    /// Get the verification queue name for a job type
    fn verify_queue_name(&self) -> QueueType;
}

/// Implementation of QueueNameForJobType for JobType
impl QueueNameForJobType for JobType {
    fn process_queue_name(&self) -> QueueType {
        match self {
            JobType::Snos => QueueType::SnosJobProcessing,
            JobType::Proving => QueueType::ProvingJobProcessing,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobProcessing,
            JobType::DataSubmission => QueueType::DataSubmissionJobProcessing,
            JobType::UpdateState => QueueType::UpdateStateJobProcessing,
            JobType::Custom(name) => QueueType::Custom(format!("{}_processing", name)),
        }
    }

    fn verify_queue_name(&self) -> QueueType {
        match self {
            JobType::Snos => QueueType::SnosJobVerification,
            JobType::Proving => QueueType::ProvingJobVerification,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobVerification,
            JobType::DataSubmission => QueueType::DataSubmissionJobVerification,
            JobType::UpdateState => QueueType::UpdateStateJobVerification,
            JobType::Custom(name) => QueueType::Custom(format!("{}_verification", name)),
        }
    }
}

/// Trait defining job queue operations
#[async_trait]
pub trait JobQueueClient: Send + Sync {
    /// Initialize the job queue client
    async fn init(&self) -> Result<()>;

    /// Create a queue with the specified configuration
    async fn create_queue(&self, queue_config: &QueueConfig) -> Result<()>;

    /// Send a message to a queue
    async fn send_message(&self, queue: QueueType, payload: String, delay: Option<Duration>) -> Result<String>;

    /// Receive messages from a queue
    async fn receive_messages(
        &self,
        queue: QueueType,
        max_messages: i32,
        wait_time: Option<Duration>,
    ) -> Result<Vec<Delivery>>;

    /// Delete a message from a queue (acknowledge)
    async fn delete_message(&self, queue: QueueType, receipt_handle: &str) -> Result<()>;

    /// Change the visibility timeout of a message
    async fn change_message_visibility(
        &self,
        queue: QueueType,
        receipt_handle: &str,
        visibility_timeout: Duration,
    ) -> Result<()>;

    /// Purge all messages from a queue
    async fn purge_queue(&self, queue: QueueType) -> Result<()>;

    /// Check if a queue exists
    async fn queue_exists(&self, queue: QueueType) -> Result<bool>;

    /// Send a job to a processing queue
    async fn send_job_to_process(
        &self,
        job_id: Uuid,
        job_type: JobType,
        custom_data: Option<serde_json::Value>,
    ) -> Result<String> {
        let queue = job_type.process_queue_name();
        let message = serde_json::to_string(&JobQueueMessage { id: job_id, job_type: Some(job_type), custom_data })?;
        self.send_message(queue, message, None).await
    }

    /// Send a job to a verification queue
    async fn send_job_to_verify(
        &self,
        job_id: Uuid,
        job_type: JobType,
        delay: Duration,
        custom_data: Option<serde_json::Value>,
    ) -> Result<String> {
        let queue = job_type.verify_queue_name();
        let message = serde_json::to_string(&JobQueueMessage { id: job_id, job_type: Some(job_type), custom_data })?;
        self.send_message(queue, message, Some(delay)).await
    }
}
