use aws_config::SdkConfig;
use aws_sdk_sns::Client;
use crate::params::AlertArgs;

pub struct SNS {
    client: Client,
    topic_arn: String,
}


impl SNS {
    pub async fn setup(args: &AlertArgs, aws_config: &SdkConfig) -> Self {
        Self { client: Client::new(aws_config), topic_arn: args.endpoint.clone() }
    }
}
