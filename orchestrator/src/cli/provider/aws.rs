use std::num::NonZeroU64;

use clap::Args;
use serde::Serialize;

/// AWS provider parameters.
///
/// AWS request attempts and retry mode use the SDK-native `AWS_MAX_ATTEMPTS`
/// and `AWS_RETRY_MODE` environment variables.
#[derive(Debug, Clone, Args, Serialize)]
pub struct AWSConfigCliArgs {
    /// Use this flag to enable AWS provider.
    #[arg(long)]
    pub aws: bool,
    /// The prefix value.
    /// And added to the start of each resource name if available
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_PREFIX", long, default_value = None)]
    pub aws_prefix: Option<String>,

    /// AWS HTTP connection timeout, in seconds. When unset, uses the AWS SDK default.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_CONNECT_TIMEOUT_SECS", long)]
    pub aws_connect_timeout_secs: Option<NonZeroU64>,

    /// AWS identity-provider load timeout, in seconds. When unset, uses the AWS SDK default.
    #[arg(env = "MADARA_ORCHESTRATOR_AWS_IDENTITY_LOAD_TIMEOUT_SECS", long)]
    pub aws_identity_load_timeout_secs: Option<NonZeroU64>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        aws: AWSConfigCliArgs,
    }

    #[test]
    fn aws_timeouts_are_configurable() {
        let parsed = TestCli::try_parse_from([
            "test",
            "--aws-connect-timeout-secs",
            "8",
            "--aws-identity-load-timeout-secs",
            "21",
        ])
        .unwrap();

        assert_eq!(
            (
                parsed.aws.aws_connect_timeout_secs.map(NonZeroU64::get),
                parsed.aws.aws_identity_load_timeout_secs.map(NonZeroU64::get),
            ),
            (Some(8), Some(21))
        );
    }

    #[test]
    fn aws_timeouts_reject_zero() {
        let result = TestCli::try_parse_from(["test", "--aws-connect-timeout-secs", "0"]);

        assert!(result.is_err());
    }
}
