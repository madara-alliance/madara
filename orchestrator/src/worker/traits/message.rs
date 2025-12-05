use crate::error::event::EventSystemResult;
use crate::worker::parser::job_queue_message::JobQueueMessage;
use crate::worker::parser::priority_queue_message::PriorityQueueMessage;
use crate::worker::parser::worker_trigger_message::WorkerTriggerMessage;
use omniqueue::Delivery;

#[derive(Debug, Clone)]
pub enum ParsedMessage {
    WorkerTrigger(Box<WorkerTriggerMessage>),
    JobQueue(Box<JobQueueMessage>),
    PriorityJobQueue(Box<PriorityQueueMessage>),
}

/// MessageParser - Trait to parse the message from the queue
/// This trait is used to parse the message from the queue
/// and convert it into the required format for the worker
pub trait MessageParser: Send + Sync {
    fn parse_message(message: &Delivery) -> EventSystemResult<Box<Self>>;
}
