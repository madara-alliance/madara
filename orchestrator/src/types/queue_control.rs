use std::collections::HashMap;
use std::sync::LazyLock;

use crate::types::queue::QueueType;
use crate::types::Layer;

#[derive(Clone)]
pub struct DlqConfig {
    pub max_receive_count: u32,
    #[allow(dead_code)]
    pub dlq_name: QueueType,
}

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
    pub dlq_config: Option<DlqConfig>,
    pub supported_layers: Vec<Layer>,
}

pub static QUEUES: LazyLock<HashMap<QueueType, QueueConfig>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(
        QueueType::JobHandleFailure,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: None,
            queue_control: QueueControlConfig::default(),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::WorkerTrigger,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: None,
            queue_control: QueueControlConfig::default_with_message_count(50),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::SnosJobProcessing,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::default_with_message_count(200),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::SnosJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::default_with_message_count(5),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::ProvingJobProcessing,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::ProvingJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::ProofRegistrationJobProcessing,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L3],
        },
    );
    map.insert(
        QueueType::ProofRegistrationJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L3],
        },
    );
    map.insert(
        QueueType::DataSubmissionJobProcessing,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::DataSubmissionJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::UpdateStateJobProcessing,
        QueueConfig {
            visibility_timeout: 900,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::UpdateStateJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map
});
