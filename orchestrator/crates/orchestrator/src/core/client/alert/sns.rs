use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use crate::core::client::alert::AlertClient;
use crate::OrchestratorResult;
use crate::params::AlertArgs;

pub struct SNS {
    client: Client,
    topic_arn: String,
}


impl SNS {
    pub fn setup(args: &AlertArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Client::new(aws_config), topic_arn: args.endpoint.clone() }
    }
}

impl AlertClient for SNS {
    fn create_alert(&self, topic_name: &str) -> OrchestratorResult<()> {
        todo!()
    }
    async fn get_topic_name(&self) -> String {
        todo!()
    }
    async fn send_alert_message(&self, message_body: String) -> OrchestratorResult<()> {
        todo!()
    }
}
