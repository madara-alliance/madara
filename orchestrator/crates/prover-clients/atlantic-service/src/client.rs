use std::path::PathBuf;

use cairo_vm::types::layout_name::LayoutName;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Method;
use tracing::debug;
use url::Url;

use crate::constants::{AGGREGATOR_FULL_OUTPUT, AGGREGATOR_USE_KZG_DA, ATLANTIC_PROOF_URL};
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

/// Struct to store job info
pub struct AtlanticJobInfo {
    /// Path of the Cairo PIE file
    pub pie_file: PathBuf,
    /// Number of steps
    pub n_steps: Option<usize>,
}

/// Struct to store job config
pub struct AtlanticJobConfig {
    /// Layout to be used for the proof
    pub proof_layout: LayoutName,
    /// Cairo VM to be used
    pub cairo_vm: AtlanticCairoVm,
    /// Result to be returned by the prover
    pub result: AtlanticQueryStep,
    /// Network being used
    pub network: String,
}

/// Struct to store bucket info
pub struct AtlanticBucketInfo {
    /// Bucket ID
    pub bucket_id: Option<String>,
    /// Index of the job in the bucket
    pub bucket_job_index: Option<u64>,
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
        request.form_text("result", &AtlanticQueryStep::ProofGeneration.to_string())
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

    /// Fetch an artifact from the given path
    ///
    /// # Arguments
    /// `artifact_path` - the path of the artifact to get
    ///
    /// # Returns
    /// The artifact as a byte array if the request is successful, otherwise an error is returned
    pub async fn get_artifacts(&self, artifact_path: String) -> Result<Vec<u8>, AtlanticError> {
        debug!("Getting artifacts from {}", artifact_path);
        let client = reqwest::Client::new();
        let response = client.get(&artifact_path).send().await.map_err(AtlanticError::GetJobArtifactsFailure)?;

        if response.status().is_success() {
            let response_text = response.bytes().await.map_err(AtlanticError::GetJobArtifactsFailure)?;
            Ok(response_text.to_vec())
        } else {
            Err(AtlanticError::AtlanticService(response.status()))
        }
    }

    pub async fn get_bucket(&self, bucket_id: &str) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        let response = self
            .client
            .request()
            .method(Method::GET)
            .path("buckets")
            .path(bucket_id)
            .send()
            .await
            .map_err(|e| AtlanticError::GetBucketStatusFailure(e))?;

        match response.status().is_success() {
            true => response.json().await.map_err(|e| AtlanticError::GetBucketStatusFailure(e)),
            false => Err(AtlanticError::AtlanticService(response.status())),
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
                aggregator_params: AtlanticAggregatorParams {
                    use_kzg_da: AGGREGATOR_USE_KZG_DA,
                    full_output: AGGREGATOR_FULL_OUTPUT,
                },
            })
            .map_err(AtlanticError::BodyParseError)?
            .send()
            .await
            .map_err(AtlanticError::CreateBucketFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::CreateBucketFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
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
            .map_err(AtlanticError::CloseBucketFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::CloseBucketFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
        }
    }

    /// Submits request to the prover client
    /// `bucket_id` and `bucket_job_id` are `None` for L3 (or L2 when AR is not needed)
    pub async fn add_job(
        &self,
        job_info: AtlanticJobInfo,
        job_config: AtlanticJobConfig,
        bucket_info: AtlanticBucketInfo,
        api_key: impl AsRef<str>,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let proof_layout = match job_config.proof_layout {
            LayoutName::dynamic => "dynamic",
            _ => job_config.proof_layout.to_str(),
        };

        debug!(
            "Submitting job with layout: {}, n_steps: {}, network: {}, ",
            proof_layout,
            Self::n_steps_to_job_size(job_info.n_steps),
            &job_config.network
        );

        let mut request = self.proving_layer.customize_request(
            // NOTE: Removing layout from the query params as it is unnecessary now (as conveyed by Atlantic)
            self.client
                .request()
                .method(Method::POST)
                .path("atlantic-query")
                .query_param("apiKey", api_key.as_ref())
                .form_text("declaredJobSize", Self::n_steps_to_job_size(job_info.n_steps))
                .form_text("result", &job_config.result.to_string())
                .form_text("network", job_config.network.as_ref())
                .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                .form_text("cairoVm", &job_config.cairo_vm.as_str())
                .form_file("pieFile", job_info.pie_file.as_ref(), "pie.zip", Some("application/zip"))?,
        );

        if let Some(bucket_id) = bucket_info.bucket_id {
            request = request.form_text("bucketId", &bucket_id);
        }
        if let Some(bucket_job_index) = bucket_info.bucket_job_index {
            request = request.form_text("bucketJobIndex", &bucket_job_index.to_string());
        }

        let response = request.send().await.map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
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
            Err(AtlanticError::AtlanticService(response.status()))
        }
    }

    // get_proof_by_task_id - is a endpoint to get the proof from the herodotus service
    // Args:
    // task_id - the task id of the proof to get
    // Returns:
    // The proof as a string if the request is successful, otherwise an error is returned
    pub async fn get_proof_by_task_id(&self, task_id: &str) -> Result<String, AtlanticError> {
        // Note: It seems this code will be replaced by the proper API once it is available
        debug!("Getting proof for task_id: {}", task_id);
        let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);
        let client = reqwest::Client::new();
        let response = client.get(&proof_path).send().await.map_err(AtlanticError::GetJobArtifactsFailure)?;

        if response.status().is_success() {
            let response_text = response.text().await.map_err(AtlanticError::GetJobArtifactsFailure)?;
            Ok(response_text)
        } else {
            Err(AtlanticError::AtlanticService(response.status()))
        }
    }

    pub async fn submit_l2_query(
        &self,
        proof: &str,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
        atlantic_api_key: &str,
        program_hash: &str,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        // TODO: we are having two function to atlantic query might need to merge them with appropriate argument
        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("atlantic-query")
            .query_param("apiKey", atlantic_api_key.as_ref())
            .form_file_bytes("inputFile", proof.as_bytes().to_vec(), "proof.json", Some("application/json"))?
            .form_text("programHash", program_hash)
            .form_text("layout", LayoutName::recursive_with_poseidon.to_str())
            .form_text("declaredJobSize", Self::n_steps_to_job_size(n_steps))
            .form_text("network", atlantic_network.as_ref())
            .form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
            .form_text("cairoVm", &AtlanticCairoVm::Python.as_str())
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
            .send()
            .await
            .map_err(AtlanticError::AddJobFailure)?;

        match response.status().is_success() {
            true => response.json().await.map_err(AtlanticError::AddJobFailure),
            false => Err(AtlanticError::AtlanticService(response.status())),
        }
    }

    // https://docs.herodotus.cloud/atlantic/sending-query#sending-query
    fn n_steps_to_job_size(n_steps: Option<usize>) -> &'static str {
        let n_steps = n_steps.unwrap_or(40_000_000) / 1_000_000;

        match n_steps {
            0..=12 => "S",
            13..=29 => "M",
            _ => "L",
        }
    }
}
