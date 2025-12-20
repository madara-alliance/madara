use crate::types::queue::QueueType;
use crate::types::Layer;
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone)]
pub struct QueueControlConfig {
    // Max message count is the maximum number of messages to receive from the queue.
    pub max_message_count: usize,
}

impl QueueControlConfig {
    pub fn default_with_message_count(max_message_count: usize) -> Self {
        Self { max_message_count }
    }

    pub fn new(max_message_count: usize) -> Self {
        Self { max_message_count }
    }
}

impl Default for QueueControlConfig {
    fn default() -> Self {
        Self { max_message_count: 10 }
    }
}

#[derive(Clone)]
pub struct QueueConfig {
    pub visibility_timeout: u32,
    pub queue_control: QueueControlConfig,
    pub supported_layers: Vec<Layer>,
}

// Queue configuration - now only WorkerTrigger queue exists
pub static QUEUES: LazyLock<HashMap<QueueType, QueueConfig>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(
        QueueType::WorkerTrigger,
        QueueConfig {
            visibility_timeout: 300,
            queue_control: QueueControlConfig::default_with_message_count(50),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map
});
