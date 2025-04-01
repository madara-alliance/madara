use aws_config::SdkConfig;
use aws_sdk_sqs::Client;
use url::Url;
use crate::core::client::queue::QueueClient;
use crate::OrchestratorResult;
use crate::params::QueueArgs;
use crate::queue::QueueType;
use crate::queue::sqs::AWSSQSValidatedArgs;

pub struct SQS {
    client: Client,
    queue_base_url: Url,
    sqs_prefix: String,
    sqs_suffix: String,
}


impl SQS {
    pub fn setup(params: &QueueArgs, aws_config: &SdkConfig) -> OrchestratorResult<Self> {
        let sqs_config_builder = aws_sdk_sqs::config::Builder::from(aws_config);
        let client = Client::from_conf(sqs_config_builder.build());
        Ok(Self {
            client,
            queue_base_url: Url::parse(&params.queue_base_url).expect("Failed to parse queue base URL"),
            sqs_prefix: params.prefix.clone(),
            sqs_suffix: params.suffix.clone(),
        })
    }

    pub fn get_queue_url(&self, queue_type: QueueType) -> String {
        let name = format!("{}/{}", self.queue_base_url, self.get_queue_name(queue_type));
        name
    }

    pub fn get_queue_name(&self, queue_type: QueueType) -> String {
        format!("{}_{}_{}", self.sqs_prefix, queue_type, self.sqs_suffix)
    }
}

impl QueueClient for SQS {


}