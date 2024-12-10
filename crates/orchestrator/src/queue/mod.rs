pub mod job_queue;
pub mod sqs;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use color_eyre::Result as EyreResult;
use lazy_static::lazy_static;
use mockall::automock;
use omniqueue::{Delivery, QueueError};
use strum_macros::{Display, EnumIter};

use crate::config::Config;
use crate::jobs::JobError;

#[derive(Display, Debug, Clone, PartialEq, Eq, EnumIter)]
pub enum QueueType {
    #[strum(serialize = "snos_job_processing")]
    SnosJobProcessing,
    #[strum(serialize = "snos_job_verification")]
    SnosJobVerification,
    #[strum(serialize = "proving_job_processing")]
    ProvingJobProcessing,
    #[strum(serialize = "proving_job_verification")]
    ProvingJobVerification,
    #[strum(serialize = "proof_registration_job_processing")]
    ProofRegistrationJobProcessing,
    #[strum(serialize = "proof_registration_job_verification")]
    ProofRegistrationJobVerification,
    #[strum(serialize = "data_submission_job_processing")]
    DataSubmissionJobProcessing,
    #[strum(serialize = "data_submission_job_verification")]
    DataSubmissionJobVerification,
    #[strum(serialize = "update_state_job_processing")]
    UpdateStateJobProcessing,
    #[strum(serialize = "update_state_job_verification")]
    UpdateStateJobVerification,
    #[strum(serialize = "job_handle_failure")]
    JobHandleFailure,
    #[strum(serialize = "worker_trigger")]
    WorkerTrigger,
}

#[derive(Clone)]
pub struct DlqConfig {
    pub max_receive_count: i32,
    pub dlq_name: QueueType,
}

#[derive(Clone)]
pub struct QueueConfig {
    pub name: QueueType,
    pub visibility_timeout: i32,
    pub dlq_config: Option<DlqConfig>,
}

// TODO: use QueueType::iter() or format!
lazy_static! {
    pub static ref QUEUES: Vec<QueueConfig> = vec![
        QueueConfig { name: QueueType::JobHandleFailure, visibility_timeout: 300, dlq_config: None },
        QueueConfig {
            name: QueueType::SnosJobProcessing,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::SnosJobVerification,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::ProvingJobProcessing,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::ProvingJobVerification,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::DataSubmissionJobProcessing,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::DataSubmissionJobVerification,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::UpdateStateJobProcessing,
            visibility_timeout: 900,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::UpdateStateJobVerification,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig { name: QueueType::WorkerTrigger, visibility_timeout: 300, dlq_config: None },
    ];
}

/// Queue Provider Trait
///
/// The QueueProvider trait is used to define the methods that a queue
/// should implement to be used as a queue for the orchestrator. The
/// purpose of this trait is to allow developers to use any queue of their choice.
#[automock]
#[async_trait]
pub trait QueueProvider: Send + Sync {
    async fn send_message_to_queue(&self, queue: QueueType, payload: String, delay: Option<Duration>)
    -> EyreResult<()>;
    async fn consume_message_from_queue(&self, queue: QueueType) -> std::result::Result<Delivery, QueueError>;
    async fn create_queue(&self, queue_config: &QueueConfig) -> EyreResult<()>;
    async fn setup(&self) -> EyreResult<()> {
        // Creating the queues :
        for queue in QUEUES.iter() {
            self.create_queue(queue).await?;
        }
        Ok(())
    }
}

pub async fn init_consumers(config: Arc<Config>) -> Result<(), JobError> {
    job_queue::init_consumers(config).await
}
