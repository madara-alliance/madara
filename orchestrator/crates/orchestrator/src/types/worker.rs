use omniqueue::Delivery;

#[derive(Debug)]
pub enum MessageType {
    Message(Delivery),
    NoMessage,
}

/// WorkerConfig - This struct contains the configuration for a worker.
/// It is used to pass configuration parameters to the worker at runtime.
#[derive(Debug, Clone, Default)]
pub struct WorkerConfig {}
