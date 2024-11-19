use futures::FutureExt;
use http::StatusCode;
use hyper::body::Incoming;
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::{Request, Response};
use hyper_tls::HttpsConnector;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower::retry;
use tower::Service;
use tower::{retry::Retry, timeout::Timeout};
use url::Url;

type HttpsClient = Client<HttpsConnector<HttpConnector>, String>;
type TimeoutRetryClient = Retry<RetryPolicy, Timeout<HttpsClient>>;
pub type PausedClient = PauseLayerMiddleware<TimeoutRetryClient>;
#[derive(Debug, Clone)]
pub struct GatewayProvider {
    pub(crate) client: PausedClient,
    pub(crate) gateway_url: Url,
    pub(crate) feeder_gateway_url: Url,
    pub(crate) headers: HeaderMap,
}

impl GatewayProvider {
    pub fn new(gateway_url: Url, feeder_gateway_url: Url) -> Self {
        let pause_until = Arc::new(RwLock::new(None));
        let connector = HttpsConnector::new();
        let base_client = Client::builder(TokioExecutor::new()).build::<_, String>(connector);

        let timeout_layer = Timeout::new(base_client, Duration::from_secs(20)); // Timeout after 20 seconds
        let retry_policy = RetryPolicy::new(5, Duration::from_secs(1), Arc::clone(&pause_until)); // Retry 5 times with 1 second backoff
        let retry_layer = Retry::new(retry_policy, timeout_layer);
        let client = PauseLayerMiddleware::new(retry_layer, Arc::clone(&pause_until));

        Self { client, gateway_url, feeder_gateway_url, headers: HeaderMap::new() }
    }

    pub fn new_with_headers(gateway_url: Url, feeder_gateway_url: Url, headers: &[(HeaderName, HeaderValue)]) -> Self {
        let feeder_client = Self::new(gateway_url, feeder_gateway_url);
        let headers = headers.iter().cloned().collect();

        Self { headers, ..feeder_client }
    }

    pub fn add_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.headers.insert(name, value);
    }

    pub fn remove_header(&mut self, name: HeaderName) -> Option<HeaderValue> {
        self.headers.remove(name)
    }

    pub fn starknet_alpha_mainnet() -> Self {
        Self::new(
            Url::parse("https://alpha-mainnet.starknet.io/gateway/")
                .expect("Failed to parse Starknet Alpha Mainnet gateway url. This should not fail in prod."),
            Url::parse("https://alpha-mainnet.starknet.io/feeder_gateway/")
                .expect("Failed to parse Starknet Alpha Mainnet feeder gateway url. This should not fail in prod."),
        )
    }

    pub fn starknet_alpha_sepolia() -> Self {
        Self::new(
            Url::parse("https://alpha-sepolia.starknet.io/gateway/")
                .expect("Failed to parse Starknet Alpha Sepolia gateway url. This should not fail in prod."),
            Url::parse("https://alpha-sepolia.starknet.io/feeder_gateway/")
                .expect("Failed to parse Starknet Alpha Sepolia feeder gateway url. This should not fail in prod."),
        )
    }
    pub fn starknet_integration_sepolia() -> Self {
        Self::new(
            Url::parse("https://integration-sepolia.starknet.io/gateway/")
                .expect("Failed to parse Starknet Integration Sepolia gateway url. This should not fail in prod."),
            Url::parse("https://integration-sepolia.starknet.io/feeder_gateway/").expect(
                "Failed to parse Starknet Integration Sepolia feeder gateway url. This should not fail in prod.",
            ),
        )
    }
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    max_retries: usize,
    backoff: Duration,
    pause_until: Arc<RwLock<Option<Instant>>>,
}

impl RetryPolicy {
    pub fn new(max_retries: usize, backoff: Duration, pause_until: Arc<RwLock<Option<Instant>>>) -> Self {
        RetryPolicy { max_retries, backoff, pause_until }
    }
}

impl retry::Policy<Request<String>, Response<Incoming>, Box<dyn Error + Send + Sync>> for RetryPolicy {
    type Future = Pin<Box<dyn Future<Output = Self> + Send>>;

    #[tracing::instrument(skip(self, result), fields(module = "RetryPolicy"))]
    fn retry(
        &self,
        _: &Request<String>,
        result: Result<&Response<Incoming>, &Box<dyn Error + Send + Sync>>,
    ) -> Option<Self::Future> {
        let pause_until = self.pause_until.clone();

        match result {
            Ok(response) => {
                if response.status() == StatusCode::TOO_MANY_REQUESTS {
                    let retry_after = get_retry_after(response).unwrap_or(Duration::from_secs(10)); // Default 10 seconds

                    let next_policy = self.clone();
                    let fut = async move {
                        if (*pause_until.read().await).is_none() {
                            tracing::info!(retry_after = ?retry_after, "⏳ Rate limited, retrying");
                        }

                        *pause_until.write().await = Some(Instant::now() + retry_after);

                        // wait for the retry_after duration
                        tokio::time::sleep(retry_after).await;

                        next_policy
                    }
                    .boxed();
                    Some(fut)
                } else {
                    None
                }
            }
            Err(_) if self.max_retries > 0 => {
                // If the request failed, retry after backoff duration
                let next_policy = RetryPolicy {
                    max_retries: self.max_retries - 1,
                    backoff: self.backoff,
                    pause_until: self.pause_until.clone(),
                };
                let sleep = tokio::time::sleep(self.backoff);
                let fut = async move {
                    sleep.await;
                    next_policy
                }
                .boxed();
                Some(fut)
            }
            _ => None, // No more retries
        }
    }

    fn clone_request(&self, req: &Request<String>) -> Option<Request<String>> {
        Some(req.clone())
    }
}

fn get_retry_after(response: &Response<Incoming>) -> Option<Duration> {
    if let Some(retry_after_header) = response.headers().get("Retry-After") {
        if let Ok(retry_after_str) = retry_after_header.to_str() {
            if let Ok(retry_seconds) = retry_after_str.parse::<u64>() {
                return Some(Duration::from_secs(retry_seconds));
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct PauseLayerMiddleware<S> {
    inner: S,
    pause_until: Arc<RwLock<Option<Instant>>>,
}

impl<S> PauseLayerMiddleware<S> {
    pub fn new(inner: S, pause_until: Arc<RwLock<Option<Instant>>>) -> Self {
        PauseLayerMiddleware { inner, pause_until }
    }
}

impl<S> Service<Request<String>> for PauseLayerMiddleware<S>
where
    S: Service<Request<String>, Response = Response<Incoming>, Error = Box<dyn Error + Send + Sync>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<String>) -> Self::Future {
        let pause_until = self.pause_until.clone();
        let mut inner = self.inner.clone();

        async move {
            // Check if a pause is active
            let pause_duration = {
                let maybe_pause_instant = *pause_until.read().await;
                if let Some(pause_instant) = maybe_pause_instant {
                    let now = Instant::now();
                    if pause_instant > now {
                        Some(pause_instant - now)
                    } else {
                        *pause_until.write().await = None;
                        None
                    }
                } else {
                    None
                }
            };

            if let Some(duration) = pause_duration {
                tokio::time::sleep(duration).await;
            }

            inner.call(req).await
        }
        .boxed()
    }
}
