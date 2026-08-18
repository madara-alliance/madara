use crate::cli::provider::aws::AWSConfigCliArgs;
use aws_config::timeout::TimeoutConfig;
use aws_config::SdkConfig;
use std::time::Duration;

const AWS_CONNECT_TIMEOUT_SECS_ENV: &str = "MADARA_ORCHESTRATOR_AWS_CONNECT_TIMEOUT_SECS";

#[derive(Debug, Clone)]
pub struct AWSCredentials {
    pub prefix: Option<String>,
}

impl AWSCredentials {
    pub async fn get_aws_config(&self) -> Result<SdkConfig, String> {
        let mut loader = aws_config::from_env();
        if let Some(timeout_config) = aws_connect_timeout_config(std::env::var(AWS_CONNECT_TIMEOUT_SECS_ENV).ok())? {
            loader = loader.timeout_config(timeout_config);
        }
        Ok(loader.load().await)
    }
}

fn aws_connect_timeout_config(value: Option<String>) -> Result<Option<TimeoutConfig>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let seconds = value.parse::<u64>().map_err(|_| timeout_config_error())?;
    if seconds == 0 {
        return Err(timeout_config_error());
    }

    Ok(Some(TimeoutConfig::builder().connect_timeout(Duration::from_secs(seconds)).build()))
}

fn timeout_config_error() -> String {
    format!("{AWS_CONNECT_TIMEOUT_SECS_ENV} must be a positive integer number of seconds")
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
    fn aws_connect_timeout_config_is_optional() {
        assert!(aws_connect_timeout_config(None).unwrap().is_none());
    }

    #[test]
    fn aws_connect_timeout_config_uses_env_value() {
        let timeout = aws_connect_timeout_config(Some("10".to_string())).unwrap().unwrap();
        assert_eq!(timeout.connect_timeout(), Some(Duration::from_secs(10)));
    }

    #[test]
    fn aws_connect_timeout_config_rejects_invalid_values() {
        for value in ["0", "abc"] {
            assert_eq!(
                aws_connect_timeout_config(Some(value.to_string())).unwrap_err(),
                "MADARA_ORCHESTRATOR_AWS_CONNECT_TIMEOUT_SECS must be a positive integer number of seconds"
            );
        }
    }
}
