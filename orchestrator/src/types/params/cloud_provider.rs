use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::identity::IdentityCache;
use aws_config::timeout::TimeoutConfig;
use aws_config::SdkConfig;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone, Default)]
pub struct AWSCredentials {
    pub prefix: Option<String>,
    pub aws_connect_timeout: Option<Duration>,
    pub aws_identity_load_timeout: Option<Duration>,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        let mut loader = aws_config::from_env();

        if let Some(aws_connect_timeout) = self.aws_connect_timeout {
            loader = loader.timeout_config(TimeoutConfig::builder().connect_timeout(aws_connect_timeout).build());
        }

        if let Some(aws_identity_load_timeout) = self.aws_identity_load_timeout {
            loader = loader.identity_cache(IdentityCache::lazy().load_timeout(aws_identity_load_timeout).build());
        }

        let config = loader.load().await;
        let aws_retry_config = config.retry_config();

        info!(
            aws_max_attempts = aws_retry_config.map(|retry| retry.max_attempts()),
            aws_retry_mode = ?aws_retry_config.map(|retry| retry.mode()),
            aws_connect_timeout_secs = ?self.aws_connect_timeout.map(|timeout| timeout.as_secs()),
            aws_identity_load_timeout_secs = ?self.aws_identity_load_timeout.map(|timeout| timeout.as_secs()),
            "Configured AWS client resilience"
        );

        config
    }
}

impl From<AWSConfigCliArgs> for AWSCredentials {
    fn from(args: AWSConfigCliArgs) -> Self {
        Self {
            prefix: args.aws_prefix,
            aws_connect_timeout: args.aws_connect_timeout_secs.map(|seconds| Duration::from_secs(seconds.get())),
            aws_identity_load_timeout: args
                .aws_identity_load_timeout_secs
                .map(|seconds| Duration::from_secs(seconds.get())),
        }
    }
}
