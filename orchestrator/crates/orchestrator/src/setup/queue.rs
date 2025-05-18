use crate::types::queue::QueueType;
use lazy_static::lazy_static;

#[derive(Clone)]
pub struct DlqConfig {
    pub max_receive_count: u32,
    #[allow(dead_code)]
    pub dlq_name: QueueType,
}

#[derive(Clone)]
pub struct QueueConfig {
    pub name: QueueType,
    pub visibility_timeout: u32,
    pub dlq_config: Option<DlqConfig>,
}

lazy_static! {
    pub static ref QUEUES: Vec<QueueConfig> = vec![
        QueueConfig {
            name: QueueType::JobHandleFailure,
            visibility_timeout: 300,
            dlq_config: None,
        },
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
            name: QueueType::ProofRegistrationJobProcessing,
            visibility_timeout: 300,
            dlq_config: Some(DlqConfig { max_receive_count: 5, dlq_name: QueueType::JobHandleFailure })
        },
        QueueConfig {
            name: QueueType::ProofRegistrationJobVerification,
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
        QueueConfig {
            name: QueueType::WorkerTrigger,
            visibility_timeout: 300,
            dlq_config: None
        },
    ];
}
