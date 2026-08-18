use std::num::{NonZeroU64, NonZeroUsize};

use clap::Args;
use serde::Serialize;

/// Retry policy for idempotent reads from Madara and reference nodes.
#[derive(Debug, Clone, Args, Serialize)]
pub struct UpstreamReadRetryCliArgs {
    /// Maximum attempts for each upstream read, including the initial request.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_MAX_ATTEMPTS", long, default_value = "3")]
    pub upstream_read_max_attempts: NonZeroUsize,

    /// Timeout for each upstream read attempt, in seconds.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_TIMEOUT_SECS", long, default_value = "30")]
    pub upstream_read_timeout_secs: NonZeroU64,

    /// Initial exponential-backoff delay between upstream read attempts, in milliseconds.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_INITIAL_BACKOFF_MILLIS", long, default_value = "200")]
    pub upstream_read_initial_backoff_millis: NonZeroU64,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Debug, Parser)]
    struct TestCli {
        #[command(flatten)]
        retry: UpstreamReadRetryCliArgs,
    }

    #[test]
    fn upstream_retry_is_configurable() {
        let parsed = TestCli::try_parse_from([
            "test",
            "--upstream-read-max-attempts",
            "4",
            "--upstream-read-timeout-secs",
            "45",
            "--upstream-read-initial-backoff-millis",
            "350",
        ])
        .unwrap();

        assert_eq!(
            (
                parsed.retry.upstream_read_max_attempts.get(),
                parsed.retry.upstream_read_timeout_secs.get(),
                parsed.retry.upstream_read_initial_backoff_millis.get(),
            ),
            (4, 45, 350)
        );
    }

    #[test]
    fn upstream_retry_rejects_zero_attempts() {
        let result = TestCli::try_parse_from(["test", "--upstream-read-max-attempts", "0"]);

        assert!(result.is_err());
    }
}
