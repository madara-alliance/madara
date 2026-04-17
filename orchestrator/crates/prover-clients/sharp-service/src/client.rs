use orchestrator_utils::http_client::HttpClient;
use reqwest::header::{HeaderValue, CONTENT_TYPE};
use reqwest::{Certificate, Identity, Method, StatusCode};
use url::Url;

use crate::constants::SHARP_PROGRAM_HASH_FUNCTION;
use crate::error::SharpError;
use crate::types::{SharpAddApplicativeJobRequest, SharpAddJobRequest, SharpAddJobResponse, SharpGetStatusResponse};
use crate::SharpValidatedArgs;

/// SHARP Gateway API async wrapper.
///
/// Communicates with SHARP's `/v1/gateway/` endpoints using mTLS.
pub struct SharpClient {
    client: HttpClient,
    offchain_proof: bool,
}

impl SharpClient {
    /// Create a new SHARP client with mTLS certificates.
    ///
    /// The base URL should include the gateway path prefix, e.g.
    /// `http://host:9511/v1/gateway`.
    pub fn new_with_args(url: Url, sharp_params: &SharpValidatedArgs) -> Self {
        // Cert/key fields carry raw PEM content (read from *_FILE paths by config).
        let cert = sharp_params.sharp_user_crt.as_bytes();
        let key = sharp_params.sharp_user_key.as_bytes();
        let server_cert = sharp_params.sharp_server_crt.as_bytes();

        let customer_id = sharp_params.sharp_customer_id.clone();

        let identity =
            Identity::from_pkcs8_pem(cert, key).expect("Failed to build the identity from certificate and key");
        let certificate = Certificate::from_pem(server_cert).expect("Failed to add root certificate");

        let client = HttpClient::builder(url.as_str())
            .expect("Failed to create HTTP client builder")
            .identity(identity)
            .add_root_certificate(certificate)
            .default_query_param("customer_id", customer_id.as_str())
            .build()
            .expect("Failed to build HTTP client");

        Self { client, offchain_proof: sharp_params.sharp_offchain_proof }
    }

    /// POST /add_job
    ///
    /// Submit a child CairoPIE for proving. The `cairo_pie_b64` should be the
    /// base64-encoded bytes of the CairoPIE zip file.
    pub async fn add_job(&self, cairo_pie_b64: &str, cairo_job_key: &str) -> Result<SharpAddJobResponse, SharpError> {
        let body = SharpAddJobRequest { cairo_pie_encoded: cairo_pie_b64 };

        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("add_job")
            .query_param("cairo_job_key", cairo_job_key)
            .query_param("offchain_proof", "true") // TODO: don't hardcode it
            .query_param("program_hash_function", SHARP_PROGRAM_HASH_FUNCTION)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .map_err(|e| SharpError::SerializationError(e.into()))?
            .send()
            .await
            .map_err(SharpError::AddJobFailure)?;

        match response.status() {
            StatusCode::OK => response.json().await.map_err(SharpError::AddJobFailure),
            code => {
                let url = response.url().to_string();
                // Read and log body at debug level (responses can be large;
                // don't let them pollute error messages / logs at higher levels).
                let body = response.text().await.unwrap_or_default();
                tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                Err(SharpError::SharpService { status: code, url })
            }
        }
    }

    /// POST /add_applicative_job
    ///
    /// Submit an applicative (aggregator) job that combines results from child jobs.
    /// The `cairo_pie_b64` is the base64-encoded aggregator PIE, and
    /// `children_cairo_job_keys` lists the child job keys in applicative order.
    pub async fn add_applicative_job(
        &self,
        cairo_pie_b64: &str,
        cairo_job_key: &str,
        children_cairo_job_keys: &[String],
    ) -> Result<SharpAddJobResponse, SharpError> {
        let body = SharpAddApplicativeJobRequest { cairo_pie_encoded: cairo_pie_b64, children_cairo_job_keys };

        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("add_applicative_job")
            .query_param("cairo_job_key", cairo_job_key)
            .query_param("offchain_proof", &self.offchain_proof.to_string())
            .query_param("program_hash_function", SHARP_PROGRAM_HASH_FUNCTION)
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(body)
            .map_err(|e| SharpError::SerializationError(e.into()))?
            .send()
            .await
            .map_err(SharpError::AddApplicativeJobFailure)?;

        match response.status() {
            StatusCode::OK => response.json().await.map_err(SharpError::AddApplicativeJobFailure),
            code => {
                let url = response.url().to_string();
                // Read and log body at debug level (responses can be large;
                // don't let them pollute error messages / logs at higher levels).
                let body = response.text().await.unwrap_or_default();
                tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                Err(SharpError::SharpService { status: code, url })
            }
        }
    }

    /// GET /get_status
    ///
    /// Query the current status of a job.
    pub async fn get_job_status(&self, cairo_job_key: &str) -> Result<SharpGetStatusResponse, SharpError> {
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
                // Read and log body at debug level (responses can be large;
                // don't let them pollute error messages / logs at higher levels).
                let body = response.text().await.unwrap_or_default();
                tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                Err(SharpError::SharpService { status: code, url })
            }
        }
    }

    /// GET /get_proof
    ///
    /// Retrieve the proof for a completed offchain job.
    pub async fn get_proof(&self, cairo_job_key: &str) -> Result<String, SharpError> {
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
                // Read and log body at debug level (responses can be large;
                // don't let them pollute error messages / logs at higher levels).
                let body = response.text().await.unwrap_or_default();
                tracing::debug!(status = %code, url = %url, body = %body, "SHARP non-2xx response");
                Err(SharpError::SharpService { status: code, url })
            }
        }
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
