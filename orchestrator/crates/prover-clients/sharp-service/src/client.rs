use std::time::Instant;

use orchestrator_prover_client_interface::retry::{retry_with_exponential_backoff, RetryConfig, RetryableRequestError};
use orchestrator_utils::http_client::HttpClient;
use reqwest::header::{HeaderValue, CONTENT_TYPE};
use reqwest::{Certificate, Identity, Method, StatusCode};
use url::Url;

use crate::constants::{APPLICATIVE_JOB_OFFCHAIN_PROOF, CHILD_JOB_OFFCHAIN_PROOF, SHARP_PROGRAM_HASH_FUNCTION};
use crate::error::SharpError;
use crate::metrics::SHARP_METRICS;
use crate::types::{SharpAddApplicativeJobRequest, SharpAddJobRequest, SharpAddJobResponse, SharpGetStatusResponse};
use crate::SharpValidatedArgs;

/// SHARP Gateway API async wrapper.
///
/// Communicates with SHARP's `/v1/gateway/` endpoints using mTLS.
/// All HTTP calls are automatically retried with exponential backoff
/// for transient errors (connection failures, timeouts, 5xx responses).
pub struct SharpClient {
    client: HttpClient,
    retry_config: RetryConfig,
}

impl SharpClient {
    /// Create a new SHARP client with mTLS certificates.
    ///
    /// The base URL should include the gateway path prefix, e.g.
    /// `http://host:9511/v1/gateway`.
    pub fn new_with_args(url: Url, sharp_params: &SharpValidatedArgs) -> Self {
        let cert = sharp_params.sharp_user_crt.as_bytes();
        let key = sharp_params.sharp_user_key.as_bytes();
        let server_cert = sharp_params.sharp_server_crt.as_bytes();

        let customer_id = sharp_params.sharp_customer_id.clone();

        let identity = Identity::from_pem(&combine_cert_and_key_pem(cert, key))
            .expect("Failed to build the identity from certificate and key");
        let certificate = Certificate::from_pem(server_cert).expect("Failed to add root certificate");

        // Force rustls: macOS native-tls (SecureTransport) doesn't reliably honor
        // add_root_certificate() for self-signed certs. rustls works consistently
        // across platforms. The server cert must have CA:TRUE for rustls to accept it
        // as a trust anchor (grab it from the live server if needed:
        // `openssl s_client -connect <host>:443 -showcerts </dev/null | openssl x509 -out server.crt`).
        let client = HttpClient::builder(url.as_str())
            .expect("Failed to create HTTP client builder")
            .identity(identity)
            .add_root_certificate(certificate)
            .use_rustls_tls()
            .default_query_param("customer_id", customer_id.as_str())
            .build()
            .expect("Failed to build HTTP client");

        Self { client, retry_config: RetryConfig::default() }
    }

    /// POST /add_job
    ///
    /// Submit a child CairoPIE for proving. The `cairo_pie_b64` should be the
    /// base64-encoded bytes of the CairoPIE zip file.
    pub async fn add_job(&self, cairo_pie_b64: &str, cairo_job_key: &str) -> Result<SharpAddJobResponse, SharpError> {
        let body = SharpAddJobRequest { cairo_pie_encoded: cairo_pie_b64 };
        let start = Instant::now();

        let result = retry_with_exponential_backoff("sharp_add_job", cairo_job_key, self.retry_config, || async {
            let response = self
                .client
                .request()
                .method(Method::POST)
                .path("add_job")
                .query_param("cairo_job_key", cairo_job_key)
                .query_param("offchain_proof", CHILD_JOB_OFFCHAIN_PROOF)
                .query_param("program_hash_function", SHARP_PROGRAM_HASH_FUNCTION)
                .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                .body(&body)
                .map_err(|e| SharpError::SerializationError(e.into()))?
                .send()
                .await
                .map_err(SharpError::AddJobFailure)?;

            match response.status() {
                StatusCode::OK => response.json().await.map_err(SharpError::AddJobFailure),
                code => {
                    let url = response.url().to_string();
                    let body = response.text().await.unwrap_or_default();
                    tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                    Err(SharpError::SharpService { status: code, url })
                }
            }
        })
        .await;

        let duration = start.elapsed().as_secs_f64();
        match &result {
            Ok(s) => SHARP_METRICS.record_success("add_job", duration, s.outcome.retry_count()),
            Err(f) => SHARP_METRICS.record_failure("add_job", duration, f.error.error_type(), f.outcome.retry_count()),
        }

        result.map(|s| s.value).map_err(|f| f.error)
    }

    /// POST /add_applicative_job
    ///
    /// Submit an applicative (aggregator) job that combines results from child jobs.
    pub async fn add_applicative_job(
        &self,
        cairo_pie_b64: &str,
        cairo_job_key: &str,
        children_cairo_job_keys: &[String],
    ) -> Result<SharpAddJobResponse, SharpError> {
        let body = SharpAddApplicativeJobRequest { cairo_pie_encoded: cairo_pie_b64, children_cairo_job_keys };
        let start = Instant::now();

        let result =
            retry_with_exponential_backoff("sharp_add_applicative_job", cairo_job_key, self.retry_config, || async {
                let response = self
                    .client
                    .request()
                    .method(Method::POST)
                    .path("add_applicative_job")
                    .query_param("cairo_job_key", cairo_job_key)
                    .query_param("offchain_proof", APPLICATIVE_JOB_OFFCHAIN_PROOF)
                    .query_param("program_hash_function", SHARP_PROGRAM_HASH_FUNCTION)
                    .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                    .body(&body)
                    .map_err(|e| SharpError::SerializationError(e.into()))?
                    .send()
                    .await
                    .map_err(SharpError::AddApplicativeJobFailure)?;

                match response.status() {
                    StatusCode::OK => response.json().await.map_err(SharpError::AddApplicativeJobFailure),
                    code => {
                        let url = response.url().to_string();
                        let body = response.text().await.unwrap_or_default();
                        tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                        Err(SharpError::SharpService { status: code, url })
                    }
                }
            })
            .await;

        let duration = start.elapsed().as_secs_f64();
        match &result {
            Ok(s) => SHARP_METRICS.record_success("add_applicative_job", duration, s.outcome.retry_count()),
            Err(f) => SHARP_METRICS.record_failure(
                "add_applicative_job",
                duration,
                f.error.error_type(),
                f.outcome.retry_count(),
            ),
        }

        result.map(|s| s.value).map_err(|f| f.error)
    }

    /// GET /get_status
    ///
    /// Query the current status of a job.
    pub async fn get_job_status(&self, cairo_job_key: &str) -> Result<SharpGetStatusResponse, SharpError> {
        let start = Instant::now();

        let result = retry_with_exponential_backoff("sharp_get_status", cairo_job_key, self.retry_config, || async {
            let response = self
                .client
                .request()
                .method(Method::GET)
                .path("get_status")
                .query_param("cairo_job_key", cairo_job_key)
                .send()
                .await
                .map_err(SharpError::GetJobStatusFailure)?;

            match response.status() {
                StatusCode::OK => response.json().await.map_err(SharpError::GetJobStatusFailure),
                code => {
                    let url = response.url().to_string();
                    let body = response.text().await.unwrap_or_default();
                    tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                    Err(SharpError::SharpService { status: code, url })
                }
            }
        })
        .await;

        let duration = start.elapsed().as_secs_f64();
        match &result {
            Ok(s) => SHARP_METRICS.record_success("get_status", duration, s.outcome.retry_count()),
            Err(f) => {
                SHARP_METRICS.record_failure("get_status", duration, f.error.error_type(), f.outcome.retry_count())
            }
        }

        result.map(|s| s.value).map_err(|f| f.error)
    }

    /// GET /get_proof
    ///
    /// Retrieve the proof for a completed offchain job.
    pub async fn get_proof(&self, cairo_job_key: &str) -> Result<String, SharpError> {
        let start = Instant::now();

        let result = retry_with_exponential_backoff("sharp_get_proof", cairo_job_key, self.retry_config, || async {
            let response = self
                .client
                .request()
                .method(Method::GET)
                .path("get_proof")
                .query_param("cairo_job_key", cairo_job_key)
                .send()
                .await
                .map_err(SharpError::GetProofFailure)?;

            match response.status() {
                StatusCode::OK => response.text().await.map_err(SharpError::GetProofFailure),
                code => {
                    let url = response.url().to_string();
                    let body = response.text().await.unwrap_or_default();
                    tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                    Err(SharpError::SharpService { status: code, url })
                }
            }
        })
        .await;

        let duration = start.elapsed().as_secs_f64();
        match &result {
            Ok(s) => SHARP_METRICS.record_success("get_proof", duration, s.outcome.retry_count()),
            Err(f) => {
                SHARP_METRICS.record_failure("get_proof", duration, f.error.error_type(), f.outcome.retry_count())
            }
        }

        result.map(|s| s.value).map_err(|f| f.error)
    }

    /// GET /is_alive
    ///
    /// Health check endpoint.
    pub async fn is_alive(&self) -> Result<bool, SharpError> {
        let response = self
            .client
            .request()
            .method(Method::GET)
            .path("is_alive")
            .send()
            .await
            .map_err(SharpError::GetJobStatusFailure)?;

        Ok(response.status().is_success())
    }
}

/// Combine a PEM-encoded certificate and private key into a single buffer.
///
/// rustls's `Identity::from_pem` requires cert + key in one contiguous PEM,
/// unlike native-tls which accepts them separately via `Identity::from_pkcs8_pem`.
fn combine_cert_and_key_pem(cert: &[u8], key: &[u8]) -> Vec<u8> {
    let mut combined = Vec::with_capacity(cert.len() + key.len() + 1);
    combined.extend_from_slice(cert);
    if !cert.ends_with(b"\n") {
        combined.push(b'\n');
    }
    combined.extend_from_slice(key);
    combined
}
