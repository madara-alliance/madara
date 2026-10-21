use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::{BehaviorVersion, SdkConfig};

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub prefix: Option<String>,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        // This behavior version selects the Hyper 1 / rustls 0.23 HTTPS client.
        // Older behavior versions can select the SDK's legacy TLS compatibility client.
        aws_config::defaults(BehaviorVersion::v2025_08_07()).load().await
    }
}

impl From<AWSConfigCliArgs> for AWSCredentials {
    fn from(args: AWSConfigCliArgs) -> Self {
        Self { prefix: args.aws_prefix }
    }
}
