use aws_config::SdkConfig;
use std::sync::Arc;

use crate::types::jobs::WorkerTriggerType;

/// InnerAWSEventBridge is a struct that represents an AWS EventBridge client.
pub(crate) struct InnerAWSEventBridge {
    pub(crate) eb_client: Arc<aws_sdk_eventbridge::Client>,
    pub(crate) scheduler_client: Arc<aws_sdk_scheduler::Client>,
    pub(crate) queue_client: Arc<aws_sdk_sqs::Client>,
    pub(crate) iam_client: Arc<aws_sdk_iam::Client>,
}

impl InnerAWSEventBridge {
    /// Creates a new instance of InnerAWSEventBridge with the provided AWS configuration.
    /// # Arguments
    /// * `aws_config` - The AWS configuration.
    ///
    /// # Returns
    /// * `Self` - The new instance of InnerAWSEventBridge.
    pub fn new(aws_config: &SdkConfig) -> Self {
        Self {
            eb_client: Arc::new(aws_sdk_eventbridge::Client::new(aws_config)),
            scheduler_client: Arc::new(aws_sdk_scheduler::Client::new(aws_config)),
            queue_client: Arc::new(aws_sdk_sqs::Client::new(aws_config)),
            iam_client: Arc::new(aws_sdk_iam::Client::new(aws_config)),
        }
    }

    pub fn get_trigger_name_from_trigger_type(trigger_rule_name: &str, trigger_type: &WorkerTriggerType) -> String {
        format!("{}-{}", trigger_rule_name, trigger_type.to_lowercase())
    }
}
