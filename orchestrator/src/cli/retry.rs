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
    /// The client-wide retry budget can stop retries before this limit.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_MAX_ATTEMPTS", long, default_value = "3")]
    pub upstream_read_max_attempts: NonZeroU32,

    /// Timeout for establishing each upstream connection, including DNS and TLS.
    /// Kept shorter than the total deadline so a stalled connection can be retried.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_CONNECT_TIMEOUT_SECS", long, default_value = "5")]
    pub upstream_read_connect_timeout_secs: NonZeroU64,

    /// Overall timeout for each upstream read, in seconds.
    #[arg(env = "MADARA_ORCHESTRATOR_UPSTREAM_READ_TIMEOUT_SECS", long, default_value = "30")]
    pub upstream_read_timeout_secs: NonZeroU64,
}

impl UpstreamReadRetryCliArgs {
    pub(crate) fn build_http_client(&self, url: &Url) -> Result<Client> {
        self.http_client_builder(url)?.build().context("failed to build upstream HTTP client")
    }

    fn http_client_builder(&self, url: &Url) -> Result<reqwest_13::ClientBuilder> {
        let host = url.host_str().context("upstream URL must include a host")?.to_owned();
        // These clients are dedicated to idempotent reads, including JSON-RPC POSTs.
        // Keep reqwest's shared retry budget to avoid amplifying an upstream outage.
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

        Ok(Client::builder()
            .connect_timeout(Duration::from_secs(self.upstream_read_connect_timeout_secs.get()))
            .timeout(Duration::from_secs(self.upstream_read_timeout_secs.get()))
            .retry(retry_policy))
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
            "--upstream-read-connect-timeout-secs",
            "7",
        ])
        .unwrap();

        assert_eq!(
            (
                parsed.retry.upstream_read_max_attempts.get(),
                parsed.retry.upstream_read_timeout_secs.get(),
                parsed.retry.upstream_read_connect_timeout_secs.get(),
            ),
            (4, 45, 7)
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

    #[test]
    fn upstream_retry_rejects_zero_timeout() {
        assert!(TestCli::try_parse_from(["test", "--upstream-read-timeout-secs", "0"]).is_err());
        assert!(TestCli::try_parse_from(["test", "--upstream-read-connect-timeout-secs", "0"]).is_err());
    }

    #[tokio::test]
    async fn http_client_retries_rate_limited_json_rpc_reads() {
        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST)
                .path("/rpc")
                .json_body(serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"starknet_blockNumber", "params":[]}));
            then.status(429);
        });
        let retry = TestCli::try_parse_from(["test"]).unwrap().retry;
        let url = Url::parse(&server.url("/rpc")).unwrap();
        let response = retry
            .build_http_client(&url)
            .unwrap()
            .post(url)
            .json(&serde_json::json!({"jsonrpc":"2.0", "id":1, "method":"starknet_blockNumber", "params":[]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        mock.assert_calls(3);
    }

    #[tokio::test]
    async fn http_client_does_not_retry_semantic_json_rpc_errors() {
        let server = httpmock::MockServer::start();
        let body = serde_json::json!({"jsonrpc":"2.0", "id":1, "error":{"code":24,"message":"Block not found"}});
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/rpc");
            then.status(200).json_body(body.clone());
        });
        let retry = TestCli::try_parse_from(["test"]).unwrap().retry;
        let url = Url::parse(&server.url("/rpc")).unwrap();
        let response = retry
            .build_http_client(&url)
            .unwrap()
            .post(url)
            .json(&serde_json::json!({"method":"starknet_getBlockWithTxHashes"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.json::<serde_json::Value>().await.unwrap(), body);
        mock.assert_calls(1);
    }

    #[tokio::test]
    async fn http_client_does_not_retry_permanent_http_errors() {
        for status in [400, 401, 403, 404] {
            let server = httpmock::MockServer::start();
            let mock = server.mock(|when, then| {
                when.method(httpmock::Method::GET).path("/read");
                then.status(status);
            });
            let retry = TestCli::try_parse_from(["test"]).unwrap().retry;
            let url = Url::parse(&server.url("/read")).unwrap();
            let response = retry.build_http_client(&url).unwrap().get(url).send().await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            mock.assert_calls(1);
        }
    }

    #[tokio::test]
    async fn http_client_one_attempt_disables_retries() {
        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/read");
            then.status(503);
        });
        let retry = TestCli::try_parse_from(["test", "--upstream-read-max-attempts", "1"]).unwrap().retry;
        let url = Url::parse(&server.url("/read")).unwrap();
        assert_eq!(
            retry.build_http_client(&url).unwrap().get(url).send().await.unwrap().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        mock.assert_calls(1);
    }

    #[tokio::test]
    async fn http_client_bounds_slow_requests_by_total_timeout() {
        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path("/slow");
            then.status(503).delay(Duration::from_secs(3));
        });
        let retry = TestCli::try_parse_from(["test", "--upstream-read-timeout-secs", "1"]).unwrap().retry;
        let url = Url::parse(&server.url("/slow")).unwrap();
        let result =
            tokio::time::timeout(Duration::from_secs(2), retry.build_http_client(&url).unwrap().get(url).send()).await;
        assert!(result.expect("overall request deadline must bound retry time").unwrap_err().is_timeout());
        mock.assert_calls(1);
    }

    #[tokio::test]
    async fn http_client_recovers_after_a_transport_failure() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = Url::parse(&format!("http://{}/read", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).await.unwrap() > 0);
                if attempt == 1 {
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                        .await
                        .unwrap();
                }
                // The first connection closes without a response, like an upstream reset.
            }
        });
        let retry = TestCli::try_parse_from(["test"]).unwrap().retry;
        let response =
            tokio::time::timeout(Duration::from_secs(5), retry.build_http_client(&url).unwrap().get(url).send())
                .await
                .unwrap()
                .unwrap();
        assert_eq!(response.text().await.unwrap(), "ok");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn http_client_retries_a_stalled_connection_before_total_deadline() {
        use reqwest_13::dns::{Addrs, Name, Resolve, Resolving};
        use std::net::SocketAddr;
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        struct StallFirstLookup {
            calls: AtomicUsize,
            address: SocketAddr,
        }
        impl Resolve for StallFirstLookup {
            fn resolve(&self, _: Name) -> Resolving {
                let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
                let address = self.address;
                Box::pin(async move {
                    if first {
                        std::future::pending::<()>().await;
                    }
                    Ok(Box::new(std::iter::once(address)) as Addrs)
                })
            }
        }

        let server = httpmock::MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::POST).path("/rpc");
            then.status(200).body("recovered");
        });
        let resolver = Arc::new(StallFirstLookup { calls: AtomicUsize::new(0), address: *server.address() });
        let retry = TestCli::try_parse_from([
            "test",
            "--upstream-read-connect-timeout-secs",
            "1",
            "--upstream-read-timeout-secs",
            "5",
        ])
        .unwrap()
        .retry;
        let url = Url::parse(&format!("http://upstream.invalid:{}/rpc", server.port())).unwrap();
        let client =
            retry.http_client_builder(&url).unwrap().no_proxy().dns_resolver(resolver.clone()).build().unwrap();
        let response = tokio::time::timeout(Duration::from_secs(4), client.post(url).body("read-only RPC").send())
            .await
            .expect("connection retry must finish before the total deadline")
            .unwrap();
        assert_eq!(response.text().await.unwrap(), "recovered");
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
        mock.assert_calls(1);
    }
}
