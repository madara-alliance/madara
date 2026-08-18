use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::identity::IdentityCache;
use aws_config::retry::RetryConfig;
use aws_config::SdkConfig;
use std::time::Duration;

// The SDK defaults to three attempts. Two additional attempts keep brief shared
// endpoint failures inside the worker instead of relying on queue redelivery.
const AWS_MIN_RETRY_ATTEMPTS: u32 = 5;
// The default five-second identity timeout can interrupt the credential
// provider before its own connection retries and backoff finish.
const AWS_IDENTITY_LOAD_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub prefix: Option<String>,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> SdkConfig {
        let config = aws_config::from_env()
            .identity_cache(IdentityCache::lazy().load_timeout(AWS_IDENTITY_LOAD_TIMEOUT).build())
            .load()
            .await;

        with_minimum_retry_attempts(config)
    }
}

fn with_minimum_retry_attempts(config: SdkConfig) -> SdkConfig {
    let retry_config = config.retry_config().cloned().unwrap_or_else(RetryConfig::standard);

    if retry_config.max_attempts() >= AWS_MIN_RETRY_ATTEMPTS {
        config
    } else {
        config.into_builder().retry_config(retry_config.with_max_attempts(AWS_MIN_RETRY_ATTEMPTS)).build()
    }
}

impl From<AWSConfigCliArgs> for AWSCredentials {
    fn from(args: AWSConfigCliArgs) -> Self {
        Self { prefix: args.aws_prefix }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_config_raises_default_retry_attempts() {
        let config = SdkConfig::builder().retry_config(RetryConfig::standard()).build();

        let config = with_minimum_retry_attempts(config);

        assert_eq!(config.retry_config().map(RetryConfig::max_attempts), Some(AWS_MIN_RETRY_ATTEMPTS));
    }

    #[test]
    fn aws_config_preserves_higher_retry_attempts() {
        let configured_attempts = AWS_MIN_RETRY_ATTEMPTS + 2;
        let config =
            SdkConfig::builder().retry_config(RetryConfig::adaptive().with_max_attempts(configured_attempts)).build();

        let config = with_minimum_retry_attempts(config);

        assert_eq!(config.retry_config().map(RetryConfig::max_attempts), Some(configured_attempts));
    }
}
