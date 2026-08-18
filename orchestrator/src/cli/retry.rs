use std::num::{NonZeroU32, NonZeroU64};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Args;
use reqwest_13::{retry, Client, StatusCode};
use serde::Serialize;
use url::Url;

/// Retry policy for idempotent reads from Madara and reference nodes.
#[derive(Debug, Clone, Copy, Args, Serialize)]
pub struct UpstreamReadRetryCliArgs {
    /// Maximum attempts for each upstream read, including the initial request.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_MAX_ATTEMPTS", long, default_value = "3")]
    pub upstream_read_max_attempts: NonZeroU32,

    /// Overall timeout for each upstream read, in seconds.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_TIMEOUT_SECS", long, default_value = "30")]
    pub upstream_read_timeout_secs: NonZeroU64,
}

impl UpstreamReadRetryCliArgs {
    pub(crate) fn build_http_client(&self, url: &Url) -> Result<Client> {
        let host = url.host_str().context("upstream URL must include a host")?.to_owned();
        let retry_policy = retry::for_host(host)
            .max_retries_per_request(self.upstream_read_max_attempts.get() - 1)
            .classify_fn(|request| {
                let retryable = request.error().is_some()
                    || request
                        .status()
                        .is_some_and(|status| status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error());

                if retryable {
                    request.retryable()
                } else {
                    request.success()
                }
            });

        Client::builder()
            .timeout(Duration::from_secs(self.upstream_read_timeout_secs.get()))
            .retry(retry_policy)
            .build()
            .context("failed to build upstream HTTP client")
    }
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
        ])
        .unwrap();

        assert_eq!(
            (parsed.retry.upstream_read_max_attempts.get(), parsed.retry.upstream_read_timeout_secs.get()),
            (4, 45)
        );
    }

    #[test]
    fn upstream_retry_rejects_zero_attempts() {
        let result = TestCli::try_parse_from(["test", "--upstream-read-max-attempts", "0"]);

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn http_client_retries_server_errors_up_to_configured_attempts() {
        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/retry");
            then.status(503);
        });
        let retry = TestCli::try_parse_from(["test"]).unwrap().retry;
        let url = Url::parse(&server.url("/retry")).unwrap();

        retry.build_http_client(&url).unwrap().get(url).send().await.unwrap();

        mock.assert_calls(3);
    }
}
