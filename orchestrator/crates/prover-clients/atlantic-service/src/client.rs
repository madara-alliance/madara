use std::path::Path;

use cairo_vm::types::layout_name::LayoutName;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Method;
use tracing::debug;
use url::Url;

use crate::error::AtlanticError;
use crate::types::{
    AtlanticAddJobResponse, AtlanticAggregatorParams, AtlanticAggregatorVersion, AtlanticBucketResponse,
    AtlanticCairoVersion, AtlanticCairoVm, AtlanticCreateBucketRequest, AtlanticGetBucketResponse,
    AtlanticGetStatusResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;

#[derive(Debug, strum_macros::EnumString)]
enum ProverType {
    #[strum(serialize = "starkware")]
    Starkware,
    #[strum(serialize = "herodotus")]
    HeroDotus,
}

trait ProvingLayer: Send + Sync {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a>;
}

struct EthereumLayer;
impl ProvingLayer for EthereumLayer {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a> {
        request
    }
}

struct StarknetLayer;
impl ProvingLayer for StarknetLayer {
    fn customize_request<'a>(&self, request: RequestBuilder<'a>) -> RequestBuilder<'a> {
        request.form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
    }
}

/// SHARP API async wrapper
pub struct AtlanticClient {
    client: HttpClient,
    proving_layer: Box<dyn ProvingLayer>,
}

impl AtlanticClient {
    /// We need to set up the client with the API_KEY.
    pub fn new_with_args(url: Url, atlantic_params: &AtlanticValidatedArgs) -> Self {
        let mock_fact_hash = atlantic_params.atlantic_mock_fact_hash.clone();
        let client = HttpClient::builder(url.as_str())
            .expect("Failed to create HTTP client builder")
            .default_form_data("mockFactHash", &mock_fact_hash)
            .build()
            .expect("Failed to build HTTP client");

        let proving_layer: Box<dyn ProvingLayer> = match atlantic_params.atlantic_settlement_layer.as_str() {
            "ethereum" => Box::new(EthereumLayer),
            "starknet" => Box::new(StarknetLayer),
            _ => panic!("Invalid settlement layer: {}", atlantic_params.atlantic_settlement_layer),
        };

        Self { client, proving_layer }
    }

    pub async fn get_artifacts(&self, artifact_path: String) -> Result<Vec<u8>, AtlanticError> {
        debug!("Getting artifacts from {}", artifact_path);
        let client = reqwest::Client::new();
        let response = client.get(&artifact_path).send().await.map_err(AtlanticError::GetJobResultFailure)?;

        if response.status().is_success() {
            let response_text = response.bytes().await.map_err(AtlanticError::GetJobResultFailure)?;
            Ok(response_text.to_vec())
        } else {
            Err(AtlanticError::SharpService(response.status()))
        }
    }

    pub async fn get_bucket(&self, bucket_id: &str) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        let response =
            self.client.request().method(Method::GET).path("buckets").path(bucket_id).send().await.map_err(|e| {
                tracing::error!("Failed to get bucket status, {}", e);
                AtlanticError::GetBucketStatusFailure(e)
            })?;

        match response.status().is_success() {
            true => response.json().await.map_err(|e| {
                tracing::error!("Failed to parse bucket status, {}", e);
                AtlanticError::GetBucketStatusFailure(e)
            }),
            false => Err(AtlanticError::SharpService(response.status())),
        }
    }

    pub async fn create_bucket(
        &self,
        atlantic_api_key: impl AsRef<str>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        let response = self
            .client
            .request()
            .method(Method::POST)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .path("buckets")
            .query_param("apiKey", atlantic_api_key.as_ref())
            .body(AtlanticCreateBucketRequest {
                external_id: None,
                node_width: None,
                aggregator_version: AtlanticAggregatorVersion::SnosAggregator0_13_3,
                aggregator_params: AtlanticAggregatorParams { use_kzg_da: true, full_output: false },
            })
            .map_err(AtlanticError::CreateBucketFailure)?
            .send()
            .await
            .map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::SharpService(response.status())),
        }
    }

    pub async fn close_bucket(
        &self,
        bucket_id: &str,
        atlantic_api_key: impl AsRef<str>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        let response = self
            .client
            .request()
            .method(Method::POST)
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .path("buckets")
            .path("close")
            .query_param("bucketId", bucket_id)
            .query_param("apiKey", atlantic_api_key.as_ref())
            .send()
            .await
            .map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::SharpService(response.status())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// Submits request to the prover client
    /// `bucket_id` and `bucket_job_id` are `None` for L3 (or L2 when AR is not needed)
    pub async fn add_job(
        &self,
        pie_file: &Path,
        proof_layout: LayoutName,
        cairo_vm: AtlanticCairoVm,
        result: AtlanticQueryStep,
        atlantic_api_key: impl AsRef<str>,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
        bucket_id: Option<String>,
        bucket_job_index: Option<u64>,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let proof_layout = match proof_layout {
            LayoutName::dynamic => "dynamic",
            _ => proof_layout.to_str(),
        };

        let mut request = self.proving_layer.customize_request(
            self.client
                .request()
                .method(Method::POST)
                .path("atlantic-query")
                .query_param("apiKey", atlantic_api_key.as_ref())
                .form_text("declaredJobSize", self.n_steps_to_job_size(n_steps))
                .form_text("layout", proof_layout)
                .form_text("result", &result.to_string())
                .form_text("network", atlantic_network.as_ref())
                .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                .form_text("cairoVm", &cairo_vm.as_str())
                .form_file("pieFile", pie_file, "pie.zip")?,
        );

        // TODO: check if we can add this in customize_request
        // current problem is that we need to pass the bucket_id and bucket_job_id as arguments
        // think of a function signature that can be used for both ethereum and starknet
        if let Some(bucket_id) = bucket_id {
            request = request.form_text("bucketId", &bucket_id);
        }
        if let Some(bucket_job_index) = bucket_job_index {
            request = request.form_text("bucketJobIndex", &bucket_job_index.to_string());
        }
        let response = request.send().await.map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::SharpService(response.status())),
        }
    }

    pub async fn get_job_status(&self, job_key: &str) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        let response = self
            .client
            .request()
            .method(Method::GET)
            .path("atlantic-query")
            .path(job_key)
            .send()
            .await
            .map_err(AtlanticError::GetJobStatusFailure)?;

        if response.status().is_success() {
            response.json().await.map_err(AtlanticError::GetJobStatusFailure)
        } else {
            Err(AtlanticError::SharpService(response.status()))
        }
    }

    // https://docs.herodotus.cloud/atlantic/sending-query#sending-query
    fn n_steps_to_job_size(&self, n_steps: Option<usize>) -> &'static str {
        let n_steps = n_steps.unwrap_or(40_000_000) / 1_000_000;

        match n_steps {
            0..=12 => "S",
            13..=29 => "M",
            _ => "L",
        }
    }
}
