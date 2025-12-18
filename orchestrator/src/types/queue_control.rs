use crate::types::queue::QueueType;
use crate::types::Layer;
use orchestrator_utils::env_utils::get_env_var_or_default;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Maximum number of messages allowed in the priority queue
/// Can be configured via MADARA_ORCHESTRATOR_MAX_PRIORITY_QUEUE_SIZE environment variable
pub static MAX_PRIORITY_QUEUE_SIZE: LazyLock<usize> = LazyLock::new(|| {
    get_env_var_or_default("MADARA_ORCHESTRATOR_MAX_PRIORITY_QUEUE_SIZE", "20")
        .parse()
        .expect("MADARA_ORCHESTRATOR_MAX_PRIORITY_QUEUE_SIZE must be a valid integer")
});

/// Maximum time (in seconds) to wait for the priority slot to become empty.
/// Can be configured via MADARA_ORCHESTRATOR_PRIORITY_SLOT_WAIT_TIMEOUT environment variable
pub static PRIORITY_SLOT_WAIT_TIMEOUT_SECS: LazyLock<u64> = LazyLock::new(|| {
    get_env_var_or_default("MADARA_ORCHESTRATOR_PRIORITY_SLOT_WAIT_TIMEOUT", "300")
        .parse()
        .expect("MADARA_ORCHESTRATOR_PRIORITY_SLOT_WAIT_TIMEOUT must be a valid integer")
});

/// Maximum time (in seconds) a message can sit in the priority slot before being
/// considered stale. Stale messages are NACKed to enable DLQ flow.
/// Can be configured via MADARA_ORCHESTRATOR_PRIORITY_SLOT_STALENESS_TIMEOUT environment variable
pub static PRIORITY_SLOT_STALENESS_TIMEOUT_SECS: LazyLock<u64> = LazyLock::new(|| {
    get_env_var_or_default("MADARA_ORCHESTRATOR_PRIORITY_SLOT_STALENESS_TIMEOUT", "300")
        .parse()
        .expect("MADARA_ORCHESTRATOR_PRIORITY_SLOT_STALENESS_TIMEOUT must be a valid integer")
});

/// Interval (in milliseconds) between priority slot availability checks.
/// Can be configured via MADARA_ORCHESTRATOR_PRIORITY_SLOT_CHECK_INTERVAL_MS environment variable
pub static PRIORITY_SLOT_CHECK_INTERVAL_MS: LazyLock<u64> = LazyLock::new(|| {
    get_env_var_or_default("MADARA_ORCHESTRATOR_PRIORITY_SLOT_CHECK_INTERVAL_MS", "1000")
        .parse()
        .expect("MADARA_ORCHESTRATOR_PRIORITY_SLOT_CHECK_INTERVAL_MS must be a valid integer")
});

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

// TODO: this should be dynamically created based on the run command params.
// So that we can skip parsing envs here again
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
            queue_control: QueueControlConfig::default_with_message_count(
                get_env_var_or_default("MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS", "5").parse().expect("MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS does not have correct value. Should be a whole number"),
            ),
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
            queue_control: QueueControlConfig::default_with_message_count(
                get_env_var_or_default("MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS", "10").parse().expect("MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS does not have correct value. Should be a whole number"),
            ),
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
            supported_layers: vec![Layer::L3],
        },
    );
    map.insert(
        QueueType::DataSubmissionJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L3],
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
    map.insert(
        QueueType::AggregatorJobProcessing,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2],
        },
    );
    map.insert(
        QueueType::AggregatorJobVerification,
        QueueConfig {
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(10),
            supported_layers: vec![Layer::L2],
        },
    );
    map.insert(
        QueueType::PriorityProcessingQueue,
        QueueConfig {
            visibility_timeout: 600, // 2x expected max processing time for slot wait + processing
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(1), // Single reader (PQ Worker)
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map.insert(
        QueueType::PriorityVerificationQueue,
        QueueConfig {
            visibility_timeout: 600, // 2x expected max processing time for slot wait + processing
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::new(1), // Single reader (PQ Worker)
            supported_layers: vec![Layer::L2, Layer::L3],
        },
    );
    map
});
