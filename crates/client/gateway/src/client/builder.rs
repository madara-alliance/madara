use hyper::client::HttpConnector;
use hyper::header::{HeaderMap, HeaderName, HeaderValue};
use hyper::{Body, Client};
use hyper::{Request, Response};
use hyper_tls::HttpsConnector;
use std::error::Error;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tower::retry::Policy;
use tower::Service;
use tower::{retry::Retry, timeout::Timeout};
use url::Url;

#[derive(Debug, Clone)]
pub struct FeederClient {
    pub(crate) client: PauseLayerMiddleware<Retry<RetryPolicy, Timeout<Client<HttpsConnector<HttpConnector>, Body>>>>,
    #[allow(dead_code)]
    pub(crate) gateway_url: Url,
    pub(crate) feeder_gateway_url: Url,
    pub(crate) headers: HeaderMap,
    pause_until: Arc<RwLock<Option<Instant>>>, // Locker for global pause in rate-limiting
}

impl FeederClient {
    pub fn new(gateway_url: Url, feeder_gateway_url: Url) -> Self {
        let pause_until = Arc::new(RwLock::new(None));
        let connector = HttpsConnector::new();
        let base_client = Client::builder().build::<_, Body>(connector);

        let timeout_layer = Timeout::new(base_client, Duration::from_secs(20)); // Timeout after 20 seconds
        let retry_policy = RetryPolicy::new(5, Duration::from_secs(1), Arc::clone(&pause_until)); // Retry 5 times with 1 second backoff
        let retry_layer = Retry::new(retry_policy, timeout_layer);
        let client = PauseLayerMiddleware::new(retry_layer, Arc::clone(&pause_until));

        Self { client, gateway_url, feeder_gateway_url, headers: HeaderMap::new(), pause_until }
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

    pub async fn request_with_pause_handling<F, R>(&self, f: F) -> Result<R, Box<dyn Error + Send + Sync>>
    where
        F: FnOnce() -> R,
    {
        let pause_until = self.pause_until.clone();

        // Check if a pause is active
        let pause_duration = {
            let lock = pause_until.read().await;
            if let Some(pause_instant) = *lock {
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

        // Wait for the pause duration if needed
        if let Some(duration) = pause_duration {
            tokio::time::sleep(duration).await;
        }

        Ok(f())
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

impl<Req> Policy<Req, Response<Body>, Box<dyn Error + Send + Sync>> for RetryPolicy
where
    Req: Clone,
{
    type Future = Pin<Box<dyn Future<Output = Self> + Send>>;

    fn retry(&self, _: &Req, result: Result<&Response<Body>, &Box<dyn Error + Send + Sync>>) -> Option<Self::Future> {
        let pause_until = self.pause_until.clone();

        match result {
            Ok(response) => {
                // TOO_MANY_REQUESTS
                if response.status().as_u16() == 429 {
                    let retry_after = get_retry_after(response).unwrap_or(Duration::from_secs(10)); // Default 10 seconds

                    let next_policy = self.clone();
                    let fut = async move {
                        let mut pause_lock = pause_until.write().await;
                        *pause_lock = Some(Instant::now() + retry_after);

                        // wait for the retry_after duration
                        tokio::time::sleep(retry_after).await;

                        next_policy
                    };
                    Some(Box::pin(fut))
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
                let fut = tokio::time::sleep(self.backoff);
                Some(Box::pin(async move {
                    fut.await;
                    next_policy
                }))
            }
            _ => None, // No more retries
        }
    }

    fn clone_request(&self, req: &Req) -> Option<Req> {
        Some(req.clone())
    }
}

fn get_retry_after(response: &Response<Body>) -> Option<Duration> {
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
pub struct PauseLayer {
    pause_until: Arc<RwLock<Option<Instant>>>,
}

impl PauseLayer {
    pub fn new(pause_until: Arc<RwLock<Option<Instant>>>) -> Self {
        PauseLayer { pause_until }
    }
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

impl<S, ReqBody> Service<Request<ReqBody>> for PauseLayerMiddleware<S>
where
    S: Service<Request<ReqBody>, Response = Response<Body>, Error = Box<dyn Error + Send + Sync>>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let pause_until = self.pause_until.clone();
        let mut inner = self.inner.clone();

        let fut = async move {
            // Check if a pause is active
            let pause_duration = {
                let lock = pause_until.read().await;
                if let Some(pause_instant) = *lock {
                    let now = Instant::now();
                    if pause_instant > now {
                        Some(pause_instant - now)
                    } else {
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
        };

        Box::pin(fut)
    }
}
