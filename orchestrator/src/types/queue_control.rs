use crate::cli::service::DEFAULT_TIMEOUT_SECONDS;
use crate::types::queue::QueueType;
use crate::types::Layer;
use orchestrator_utils::env_utils::get_env_var_or_default;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Duration;

/// IMPORTANT: This value must be greater than [DEFAULT_TIMEOUT_SECONDS] for orphan healing logic to work correctly
const QUEUE_VISIBILITY_TIMEOUT_SECONDS: u32 = DEFAULT_TIMEOUT_SECONDS as u32 + 60;
const QUEUE_MAX_RECEIVE_COUNT: u32 = 5;

/// Maximum time to wait for the priority slot to become empty.
pub const PRIORITY_SLOT_WAIT_TIMEOUT: Duration = Duration::from_secs(300);

/// Maximum time a message can sit in the priority slot before being
/// considered stale. Stale messages are NACKed to enable DLQ flow.
pub const PRIORITY_SLOT_STALENESS_TIMEOUT_SECS: u64 = QUEUE_VISIBILITY_TIMEOUT_SECONDS as u64;

/// Interval between priority slot availability checks.
pub const PRIORITY_SLOT_CHECK_INTERVAL: Duration = Duration::from_millis(1000);

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
    /// Message retention period (TTL) in seconds. If None, uses SQS default (4 days).
    /// Valid values: 60 to 1,209,600 seconds.
    pub message_retention_period: Option<u32>,
}

// TODO: this should be dynamically created based on the run command params.
// So that we can skip parsing envs here again
pub static QUEUES: LazyLock<HashMap<QueueType, QueueConfig>> = LazyLock::new(|| {
    let mut map = HashMap::new();
    map.insert(
        QueueType::JobHandleFailure,
        QueueConfig {
            visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
            dlq_config: None,
            queue_control: QueueControlConfig::default(),
            supported_layers: vec![Layer::L2, Layer::L3],
            message_retention_period: None,
        },
    );
    map.insert(
        QueueType::WorkerTrigger,
        QueueConfig {
            visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
            dlq_config: None,
            queue_control: QueueControlConfig::default_with_message_count(50),
            supported_layers: vec![Layer::L2, Layer::L3],
            message_retention_period: Some(60), // 60 second TTL for worker triggers
        },
    );
    map.insert(
        QueueType::SnosJobProcessing,
        QueueConfig {
            visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
            dlq_config: Some(DlqConfig { max_receive_count: QUEUE_MAX_RECEIVE_COUNT, dlq_name: QueueType::JobHandleFailure }),
            queue_control: QueueControlConfig::default_with_message_count(
                get_env_var_or_default("MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS", "5").parse().expect("MADARA_ORCHESTRATOR_MAX_CONCURRENT_SNOS_JOBS does not have correct value. Should be a whole number"),
            ),
            supported_layers: vec![Layer::L2, Layer::L3],
            message_retention_period: None,
        },
    );
    map.insert(
        QueueType::SnosJobVerification,
        QueueConfig {
            visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
            dlq_config: Some(DlqConfig {
                max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
                dlq_name: QueueType::JobHandleFailure,
            }),
            queue_control: QueueControlConfig::default_with_message_count(5),
            supported_layers: vec![Layer::L2, Layer::L3],
            message_retention_period: None,
        },
    );
    // map.insert(
    //     QueueType::ProvingJobProcessing,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig { max_receive_count: QUEUE_MAX_RECEIVE_COUNT, dlq_name: QueueType::JobHandleFailure }),
    //         queue_control: QueueControlConfig::default_with_message_count(
    //             get_env_var_or_default("MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS", "10").parse().expect("MADARA_ORCHESTRATOR_MAX_CONCURRENT_PROVING_JOBS does not have correct value. Should be a whole number"),
    //         ),
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::ProvingJobVerification,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::ProofRegistrationJobProcessing,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::ProofRegistrationJobVerification,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::DataSubmissionJobProcessing,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::DataSubmissionJobVerification,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::UpdateStateJobProcessing,
    //     QueueConfig {
    //         visibility_timeout: 3 * QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::UpdateStateJobVerification,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::AggregatorJobProcessing,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L2],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::AggregatorJobVerification,
    //     QueueConfig {
    //         visibility_timeout: QUEUE_VISIBILITY_TIMEOUT_SECONDS,
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(10),
    //         supported_layers: vec![Layer::L2],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::PriorityProcessingQueue,
    //     QueueConfig {
    //         visibility_timeout: 2 * QUEUE_VISIBILITY_TIMEOUT_SECONDS, // 2x expected max processing time for slot wait + processing
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(1), // Single reader (PQ Worker)
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    // map.insert(
    //     QueueType::PriorityVerificationQueue,
    //     QueueConfig {
    //         visibility_timeout: 2 * QUEUE_VISIBILITY_TIMEOUT_SECONDS, // 2x expected max processing time for slot wait + processing
    //         dlq_config: Some(DlqConfig {
    //             max_receive_count: QUEUE_MAX_RECEIVE_COUNT,
    //             dlq_name: QueueType::JobHandleFailure,
    //         }),
    //         queue_control: QueueControlConfig::new(1), // Single reader (PQ Worker)
    //         supported_layers: vec![Layer::L2, Layer::L3],
    //         message_retention_period: None,
    //     },
    // );
    map
});
