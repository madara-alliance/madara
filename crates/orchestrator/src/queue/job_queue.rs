use std::future::Future;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use color_eyre::eyre::Context;
use color_eyre::Result as EyreResult;
use omniqueue::{Delivery, QueueError};
use serde::{Deserialize, Deserializer, Serialize};
use strum::Display;
use thiserror::Error;
use tokio::time::sleep;
use uuid::Uuid;

use super::QueueType;
use crate::config::Config;
use crate::jobs::types::JobType;
use crate::jobs::{handle_job_failure, process_job, verify_job, JobError, OtherError};
use crate::workers::data_submission_worker::DataSubmissionWorker;
use crate::workers::proof_registration::ProofRegistrationWorker;
use crate::workers::proving::ProvingWorker;
use crate::workers::snos::SnosWorker;
use crate::workers::update_state::UpdateStateWorker;
use crate::workers::Worker;

#[derive(Error, Debug, PartialEq)]
pub enum ConsumptionError {
    #[error("Failed to consume message from queue, error {error_msg:?}")]
    FailedToConsumeFromQueue { error_msg: String },

    #[error("Failed to handle job with id {job_id:?}. Error: {error_msg:?}")]
    FailedToHandleJob { job_id: Uuid, error_msg: String },

    #[error("Failed to spawn {worker_trigger_type:?} worker. Error: {error_msg:?}")]
    FailedToSpawnWorker { worker_trigger_type: WorkerTriggerType, error_msg: String },

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JobQueueMessage {
    pub id: Uuid,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Display)]
#[strum(serialize_all = "PascalCase")]
pub enum WorkerTriggerType {
    Snos,
    Proving,
    ProofRegistration,
    DataSubmission,
    UpdateState,
}

#[derive(Debug, Serialize, Clone)]
pub struct WorkerTriggerMessage {
    pub worker: WorkerTriggerType,
}

impl FromStr for WorkerTriggerType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Proving" => Ok(WorkerTriggerType::Proving),
            "Snos" => Ok(WorkerTriggerType::Snos),
            "ProofRegistration" => Ok(WorkerTriggerType::ProofRegistration),
            "DataSubmission" => Ok(WorkerTriggerType::DataSubmission),
            "UpdateState" => Ok(WorkerTriggerType::UpdateState),
            _ => Err(format!("Unknown WorkerTriggerType: {}", s)),
        }
    }
}

// TODO : Need to check why serde deserializer was failing here.
// TODO : Remove this custom deserializer.
/// Implemented a custom deserializer as when using serde json deserializer
/// It was unable to deserialize the response from the event trigger.
impl<'de> Deserialize<'de> for WorkerTriggerMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize, Debug)]
        struct Helper {
            worker: String,
        }
        let helper = Helper::deserialize(deserializer)?;
        Ok(WorkerTriggerMessage {
            worker: WorkerTriggerType::from_str(&helper.worker).map_err(serde::de::Error::custom)?,
        })
    }
}

#[derive(Debug)]
enum DeliveryReturnType {
    Message(Delivery),
    NoMessage,
}

pub trait QueueNameForJobType {
    fn process_queue_name(&self) -> QueueType;
    fn verify_queue_name(&self) -> QueueType;
}

impl QueueNameForJobType for JobType {
    fn process_queue_name(&self) -> QueueType {
        match self {
            JobType::SnosRun => QueueType::SnosJobProcessing,
            JobType::ProofCreation => QueueType::ProvingJobProcessing,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobProcessing,
            JobType::DataSubmission => QueueType::DataSubmissionJobProcessing,
            JobType::StateTransition => QueueType::UpdateStateJobProcessing,
        }
    }
    fn verify_queue_name(&self) -> QueueType {
        match self {
            JobType::SnosRun => QueueType::SnosJobVerification,
            JobType::ProofCreation => QueueType::ProvingJobVerification,
            JobType::ProofRegistration => QueueType::ProofRegistrationJobVerification,
            JobType::DataSubmission => QueueType::DataSubmissionJobVerification,
            JobType::StateTransition => QueueType::UpdateStateJobVerification,
        }
    }
}

pub async fn add_job_to_process_queue(id: Uuid, job_type: &JobType, config: Arc<Config>) -> EyreResult<()> {
    tracing::info!("Adding job with id {:?} to processing queue", id);
    add_job_to_queue(id, job_type.process_queue_name(), None, config).await
}

pub async fn add_job_to_verification_queue(
    id: Uuid,
    job_type: &JobType,
    delay: Duration,
    config: Arc<Config>,
) -> EyreResult<()> {
    tracing::info!("Adding job with id {:?} to verification queue", id);
    add_job_to_queue(id, job_type.verify_queue_name(), Some(delay), config).await
}

pub async fn consume_job_from_queue<F, Fut>(
    queue: QueueType,
    handler: F,
    config: Arc<Config>,
) -> Result<(), ConsumptionError>
where
    F: FnOnce(Uuid, Arc<Config>) -> Fut,
    F: Send + 'static,
    Fut: Future<Output = Result<(), JobError>> + Send,
{
    tracing::trace!(queue = %queue, "Attempting to consume job from queue");

    let delivery = get_delivery_from_queue(queue.clone(), config.clone()).await?;

    let message = match delivery {
        DeliveryReturnType::Message(message) => {
            tracing::debug!(queue = %queue, "Message received from queue");
            message
        }
        DeliveryReturnType::NoMessage => {
            tracing::debug!(queue = %queue, "No message in queue");
            return Ok(());
        }
    };

    let job_message = parse_job_message(&message)?;

    if let Some(job_message) = job_message {
        tracing::info!(queue = %queue, job_id = %job_message.id, "Processing job message");
        tokio::spawn(async move {
            match handle_job_message(job_message, message, handler, config).await {
                Ok(_) => {}
                Err(e) => log::error!("Failed to handle job message. Error: {:?}", e),
            }
        });
    } else {
        tracing::warn!(queue = %queue, "Received empty job message");
    }

    tracing::info!(queue = %queue, "Job consumption completed successfully");
    Ok(())
}

/// Function to consume the message from the worker trigger queues and spawn the worker
/// for respective message received.
pub async fn consume_worker_trigger_messages_from_queue<F, Fut>(
    queue: QueueType,
    handler: F,
    config: Arc<Config>,
) -> Result<(), ConsumptionError>
where
    F: FnOnce(Box<dyn Worker>, Arc<Config>) -> Fut,
    F: Send + 'static,
    Fut: Future<Output = color_eyre::Result<()>> + Send,
{
    tracing::debug!("Consuming from queue {:?}", queue);
    let delivery = get_delivery_from_queue(queue, Arc::clone(&config)).await?;

    let message = match delivery {
        DeliveryReturnType::Message(message) => message,
        DeliveryReturnType::NoMessage => return Ok(()),
    };

    let job_message = parse_worker_message(&message)?;

    if let Some(job_message) = job_message {
        tokio::spawn(async move {
            match handle_worker_message(job_message, message, handler, config).await {
                Ok(_) => {}
                Err(e) => tracing::error!("Failed to handle worker message. Error: {:?}", e),
            }
        });
    }

    Ok(())
}

fn parse_job_message(message: &Delivery) -> Result<Option<JobQueueMessage>, ConsumptionError> {
    message
        .payload_serde_json()
        .wrap_err("Payload Serde Error")
        .map_err(|e| ConsumptionError::Other(OtherError::from(e)))
}

/// Using string since localstack currently is instable with deserializing maps.
/// Change this to accept a map after localstack is stable
fn parse_worker_message(message: &Delivery) -> Result<Option<WorkerTriggerMessage>, ConsumptionError> {
    let payload = message
        .borrow_payload()
        .ok_or_else(|| ConsumptionError::Other(OtherError::from("Empty payload".to_string())))?;
    let message_string = String::from_utf8_lossy(payload).to_string().trim_matches('\"').to_string();
    let trigger_type = WorkerTriggerType::from_str(message_string.as_str()).expect("trigger type unwrapping failed");
    Ok(Some(WorkerTriggerMessage { worker: trigger_type }))
}

async fn handle_job_message<F, Fut>(
    job_message: JobQueueMessage,
    message: Delivery,
    handler: F,
    config: Arc<Config>,
) -> Result<(), ConsumptionError>
where
    F: FnOnce(Uuid, Arc<Config>) -> Fut,
    Fut: Future<Output = Result<(), JobError>>,
{
    tracing::info!("Handling job with id {:?}", job_message.id);

    match handler(job_message.id, config.clone()).await {
        Ok(_) => {
            message
                .ack()
                .await
                .map_err(|(e, _)| e)
                .wrap_err("Queue Error")
                .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to handle job with id {:?}. Error: {:?}", job_message.id, e);
            config
                .alerts()
                .send_alert_message(e.to_string())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;

            // not using `nack` as we dont' want retries in case of failures
            match message.ack().await {
                Ok(_) => Err(ConsumptionError::FailedToHandleJob {
                    job_id: job_message.id,
                    error_msg: "Job handling failed, message nack-ed".to_string(),
                }),
                Err(delivery_nack_error) => Err(ConsumptionError::FailedToHandleJob {
                    job_id: job_message.id,
                    error_msg: delivery_nack_error.0.to_string(),
                }),
            }
        }
    }
}

async fn handle_worker_message<F, Fut>(
    job_message: WorkerTriggerMessage,
    message: Delivery,
    handler: F,
    config: Arc<Config>,
) -> Result<(), ConsumptionError>
where
    F: FnOnce(Box<dyn Worker>, Arc<Config>) -> Fut,
    Fut: Future<Output = color_eyre::Result<()>>,
{
    let worker_handler = get_worker_handler_from_worker_trigger_type(job_message.worker.clone());

    match handler(worker_handler, config.clone()).await {
        Ok(_) => {
            message
                .ack()
                .await
                .map_err(|(e, _)| e)
                .wrap_err("Queue Error")
                .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;
            Ok(())
        }
        Err(e) => {
            tracing::error!("Failed to handle worker trigger {:?}. Error: {:?}", job_message.worker, e);
            config
                .alerts()
                .send_alert_message(e.to_string())
                .await
                .map_err(|e| ConsumptionError::Other(OtherError::from(e)))?;

            // not using `nack` as we dont' want retries in case of failures
            message.ack().await.map_err(|(e, _)| ConsumptionError::Other(OtherError::from(e.to_string())))?;
            Err(ConsumptionError::FailedToSpawnWorker {
                worker_trigger_type: job_message.worker,
                error_msg: "Worker handling failed, message nack-ed".to_string(),
            })
        }
    }
}

/// To get Box<dyn Worker> handler from `WorkerTriggerType`.
fn get_worker_handler_from_worker_trigger_type(worker_trigger_type: WorkerTriggerType) -> Box<dyn Worker> {
    match worker_trigger_type {
        WorkerTriggerType::Snos => Box::new(SnosWorker),
        WorkerTriggerType::Proving => Box::new(ProvingWorker),
        WorkerTriggerType::DataSubmission => Box::new(DataSubmissionWorker),
        WorkerTriggerType::ProofRegistration => Box::new(ProofRegistrationWorker),
        WorkerTriggerType::UpdateState => Box::new(UpdateStateWorker),
    }
}

/// To get the delivery from the message queue using the queue name
async fn get_delivery_from_queue(
    queue: QueueType,
    config: Arc<Config>,
) -> Result<DeliveryReturnType, ConsumptionError> {
    match config.queue().consume_message_from_queue(queue).await {
        Ok(d) => Ok(DeliveryReturnType::Message(d)),
        Err(QueueError::NoData) => Ok(DeliveryReturnType::NoMessage),
        Err(e) => Err(ConsumptionError::FailedToConsumeFromQueue { error_msg: e.to_string() }),
    }
}

macro_rules! spawn_consumer {
    ($queue_type:expr, $handler:expr, $consume_function:expr, $config:expr) => {
        let config_clone = $config.clone();
        tokio::spawn(async move {
            loop {
                match $consume_function($queue_type, $handler, config_clone.clone()).await {
                    Ok(_) => {}
                    Err(e) => tracing::error!("Failed to consume from queue {:?}. Error: {:?}", $queue_type, e),
                }
                sleep(Duration::from_millis(500)).await;
            }
        });
    };
}

pub async fn init_consumers(config: Arc<Config>) -> Result<(), JobError> {
    spawn_consumer!(QueueType::SnosJobProcessing, process_job, consume_job_from_queue, config.clone());
    spawn_consumer!(QueueType::SnosJobVerification, verify_job, consume_job_from_queue, config.clone());

    spawn_consumer!(QueueType::ProvingJobProcessing, process_job, consume_job_from_queue, config.clone());
    spawn_consumer!(QueueType::ProvingJobVerification, verify_job, consume_job_from_queue, config.clone());

    spawn_consumer!(QueueType::DataSubmissionJobProcessing, process_job, consume_job_from_queue, config.clone());
    spawn_consumer!(QueueType::DataSubmissionJobVerification, verify_job, consume_job_from_queue, config.clone());

    spawn_consumer!(QueueType::UpdateStateJobProcessing, process_job, consume_job_from_queue, config.clone());
    spawn_consumer!(QueueType::UpdateStateJobVerification, verify_job, consume_job_from_queue, config.clone());

    spawn_consumer!(QueueType::JobHandleFailure, handle_job_failure, consume_job_from_queue, config.clone());

    spawn_consumer!(QueueType::WorkerTrigger, spawn_worker, consume_worker_trigger_messages_from_queue, config);
    Ok(())
}

/// To spawn the worker by passing the worker struct
async fn spawn_worker(worker: Box<dyn Worker>, config: Arc<Config>) -> color_eyre::Result<()> {
    if let Err(e) = worker.run_worker_if_enabled(config).await {
        log::error!("Failed to spawn worker. Error: {}", e);
        return Err(e);
    }
    Ok(())
}
async fn add_job_to_queue(id: Uuid, queue: QueueType, delay: Option<Duration>, config: Arc<Config>) -> EyreResult<()> {
    let message = JobQueueMessage { id };
    config.queue().send_message_to_queue(queue.clone(), serde_json::to_string(&message)?, delay).await?;
    tracing::info!(
        log_type = "JobQueue",
        category = "add_job_to_queue",
        function_type = "add_job_to_queue",
        "Added job with id {:?} to {:?} queue",
        id,
        queue
    );
    Ok(())
}
