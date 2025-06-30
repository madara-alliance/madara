use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::SdkConfig;

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub prefix: Option<String>,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        aws_config::from_env().load().await
    }
}

impl From<AWSConfigCliArgs> for AWSCredentials {
    fn from(args: AWSConfigCliArgs) -> Self {
        Self { prefix: args.aws_prefix }
    }
}
