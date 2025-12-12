use std::path::PathBuf;
use std::time::Duration;

use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::Felt252;
use orchestrator_utils::http_client::extract_http_error_text;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Method;
use tracing::debug;
use url::Url;

use crate::constants::{
    AGGREGATOR_FULL_OUTPUT, AGGREGATOR_USE_KZG_DA, ATLANTIC_PROOF_URL, RETRY_DELAY_MS, RETRY_MAX_ATTEMPTS,
};
use crate::error::AtlanticError;
use crate::types::{
    AtlanticAddJobResponse, AtlanticAggregatorParams, AtlanticAggregatorVersion, AtlanticBucketResponse,
    AtlanticCairoVersion, AtlanticCairoVm, AtlanticCreateBucketRequest, AtlanticGetBucketResponse,
    AtlanticGetStatusResponse, AtlanticQueriesListResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;

/// Struct to store job info
pub struct AtlanticJobInfo {
    /// Path of the Cairo PIE file
    pub pie_file: PathBuf,
    /// Number of steps
    pub n_steps: Option<usize>,
    /// External ID to identify the job (used to prevent duplicate submissions)
    pub external_id: String,
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
    /// ID of the chain from which PIE is generated
    pub chain_id_hex: Option<String>,
}

/// Struct to store bucket info
pub struct AtlanticBucketInfo {
    /// Bucket ID
    pub bucket_id: Option<String>,
    /// Index of the job in the bucket
    pub bucket_job_index: Option<u64>,
}

struct ProvingParams {
    /// Layout to be used
    layout: LayoutName,
}

trait ProvingLayer: Send + Sync {
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, params: ProvingParams) -> RequestBuilder<'a>;
}

struct EthereumLayer;
impl ProvingLayer for EthereumLayer {
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, _params: ProvingParams) -> RequestBuilder<'a> {
        request
    }
}

struct StarknetLayer;
impl ProvingLayer for StarknetLayer {
    fn add_proving_params<'a>(&self, request: RequestBuilder<'a>, params: ProvingParams) -> RequestBuilder<'a> {
        request
            .form_text("result", &AtlanticQueryStep::ProofGeneration.to_string())
            .form_text("layout", params.layout.to_str())
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

    /// Generic retry mechanism for GET queries
    /// Attempts the provided async function up to RETRY_MAX_ATTEMPTS times with RETRY_DELAY_MS delay between attempts
    async fn retry_get<F, Fut, T, E>(&self, operation_name: &str, mut f: F) -> Result<T, E>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        let mut last_error = None;

        for attempt in 1..=RETRY_MAX_ATTEMPTS {
            match f().await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    debug!("Attempt {}/{} for {} failed: {}", attempt, RETRY_MAX_ATTEMPTS, operation_name, err);
                    last_error = Some(err);

                    if attempt < RETRY_MAX_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(RETRY_DELAY_MS)).await;
                    }
                }
            }
        }

        Err(last_error.expect("At least one attempt should have been made"))
    }

    /// Fetch an artifact from the given path.
    /// This expects a URL from which it'll try to fetch the artifact.
    /// It's called by `get_artifacts` service function in `lib.rs`.
    /// Artifacts can be proof, snos output, cairo pie (aggregator's), program output, etc.
    ///
    /// # Arguments
    /// `artifact_path` - the path of the artifact to get
    ///
    /// # Returns
    /// The artifact as a byte array if the request is successful, otherwise an error is returned
    pub async fn get_artifacts(&self, artifact_path: String) -> Result<Vec<u8>, AtlanticError> {
        debug!("Atlantic Request: GET {} - getting artifacts", artifact_path);

        self.retry_get("get_artifacts", || async {
            let client = reqwest::Client::new();
            let response = client.get(&artifact_path).send().await.map_err(|e| {
                AtlanticError::GetJobArtifactsFailure { context: format!("url: {}", artifact_path), source: e }
            })?;

            let status = response.status();
            if status.is_success() {
                let response_text = response.bytes().await.map_err(|e| AtlanticError::GetJobArtifactsFailure {
                    context: format!("url: {}, reading response bytes", artifact_path),
                    source: e,
                })?;
                let artifact_size = response_text.len();
                debug!(
                    "Atlantic Response: GET {} - success (status: {}, size: {} bytes)",
                    artifact_path, status, artifact_size
                );
                Ok(response_text.to_vec())
            } else {
                let (error_text, status) = extract_http_error_text(response, "get artifacts").await;
                debug!("Atlantic Response: GET {} - error (status: {}, error: {})", artifact_path, status, error_text);
                Err(AtlanticError::AtlanticService(status, error_text))
            }
        })
        .await
    }

    /// Fetch the details of a bucket from the Atlantic client
    pub async fn get_bucket(&self, bucket_id: &str) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        debug!("Atlantic Request: GET /buckets/{} - getting bucket details", bucket_id);

        self.retry_get("get_bucket", || async {
            let response =
                self.client.request().method(Method::GET).path("buckets").path(bucket_id).send().await.map_err(
                    |e| AtlanticError::GetBucketStatusFailure {
                        context: format!("bucket_id: {}", bucket_id),
                        source: e,
                    },
                )?;

            let status = response.status();
            match status.is_success() {
                true => {
                    let bucket_response = response.json().await.map_err(|e| AtlanticError::GetBucketStatusFailure {
                        context: format!("bucket_id: {}, parsing JSON response", bucket_id),
                        source: e,
                    })?;
                    debug!("Atlantic Response: GET /buckets/{} - success (status: {})", bucket_id, status);
                    Ok(bucket_response)
                }
                false => {
                    let (error_text, status) = extract_http_error_text(response, "get bucket").await;
                    debug!(
                        "Atlantic Response: GET /buckets/{} - error (status: {}, error: {})",
                        bucket_id, status, error_text
                    );
                    Err(AtlanticError::AtlanticService(status, error_text))
                }
            }
        })
        .await
    }

    /// Search Atlantic queries with optional filters
    /// This function searches through Atlantic queries using the provided search string
    /// and optional additional filters like limit, offset, client_id, network, status, and result.
    ///
    /// # Arguments
    /// * `search_string` - The search string to filter queries
    /// * `limit` - Optional limit for the number of results (default: None)
    /// * `offset` - Optional offset for pagination (default: None)
    /// * `status` - Optional status filter (default: None)
    /// * `result` - Optional result filter (default: None)
    ///
    /// # Returns
    /// Returns a list of Atlantic queries matching the search criteria and total count
    pub async fn search_atlantic_queries(
        &self,
        search_string: impl AsRef<str>,
        limit: Option<u32>,
        offset: Option<u32>,
        status: Option<&str>,
        result: Option<&str>,
    ) -> Result<AtlanticQueriesListResponse, AtlanticError> {
        debug!(
            "Atlantic Request: GET /atlantic-queries - searching queries (search: {}, limit: {:?}, offset: {:?})",
            search_string.as_ref(), limit, offset
        );

        self.retry_get("search_atlantic_queries", || async {
            let mut request = self.client.request()
                .method(Method::GET)
                .path("atlantic-queries")
                .query_param("search", search_string.as_ref());

            // Add optional query parameters
            if let Some(limit_val) = limit {
                request = request.query_param("limit", &limit_val.to_string());
            }
            if let Some(offset_val) = offset {
                request = request.query_param("offset", &offset_val.to_string());
            }
            if let Some(status_val) = status {
                request = request.query_param("status", status_val);
            }
            if let Some(result_val) = result {
                request = request.query_param("result", result_val);
            }

            let response = request.send().await.map_err(|e| AtlanticError::GetQueriesFailure {
                context: format!("search: {}", search_string.as_ref()),
                source: e,
            })?;

            let status = response.status();
            match status.is_success() {
                true => {
                    let queries_response = response.json().await.map_err(|e| AtlanticError::GetQueriesFailure {
                        context: format!("search: {}, parsing JSON response", search_string.as_ref()),
                        source: e,
                    })?;
                    debug!(
                        "Atlantic Response: GET /atlantic-queries - success (status: {}, search: {})",
                        status, search_string.as_ref()
                    );
                    Ok(queries_response)
                }
                false => {
                    let (error_text, status) = extract_http_error_text(response, "search atlantic queries").await;
                    debug!(
                        "Atlantic Response: GET /atlantic-queries - error (status: {}, error: {}, search: {})",
                        status, error_text, search_string.as_ref()
                    );
                    Err(AtlanticError::AtlanticService(status, error_text))
                }
            }
        })
        .await
    }

    /// Create a new bucket for Applicative Recursion.
    /// Initially, the bucket will be empty when created.
    /// The `bucket_id` returned from here will be used to add child jobs to this bucket.
    /// A new bucket is created when creating a new batch in Batching worker.
    ///
    // TODO(Mohit,01/12/2025): We should have an AggregatorInput struct here for args, based on:
    // https://github.com/starkware-libs/sequencer/blob/main-v0.14.1/crates/starknet_os/src/hint_processor/aggregator_hint_processor.rs#L42-L52
    // For 0.14.1, we would need to send the public_keys as well.
    pub async fn create_bucket(
        &self,
        atlantic_api_key: impl AsRef<str>,
        mock_proof: bool,
        chain_id_hex: Option<String>,
        fee_token_address: Option<Felt252>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        debug!(
            "Atlantic Request: POST /buckets - creating bucket (mock_proof: {}, chain_id_hex: {:?})",
            mock_proof, chain_id_hex
        );

        // TODO(prakhar,19/11/2025): Use the aggregator version calculated from Madara Version being passed through ENV
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
                    chain_id_hex: chain_id_hex.clone(),
                    fee_token_address,
                },
                mock_proof,
            })
            .map_err(AtlanticError::BodyParseError)?
            .send()
            .await
            .map_err(|e| AtlanticError::CreateBucketFailure {
                context: format!("mock_proof: {}, chain_id_hex: {:?}", mock_proof, chain_id_hex),
                source: e,
            })?;

        let status = response.status();
        match status.is_success() {
            true => {
                let bucket_response: AtlanticBucketResponse =
                    response.json().await.map_err(|e| AtlanticError::CreateBucketFailure {
                        context: format!(
                            "mock_proof: {}, chain_id_hex: {:?}, parsing JSON response",
                            mock_proof, chain_id_hex
                        ),
                        source: e,
                    })?;
                debug!(
                    "Atlantic Response: POST /buckets - success (status: {}, bucket_id: {})",
                    status, bucket_response.atlantic_bucket.id
                );
                Ok(bucket_response)
            }
            false => {
                let (error_text, status) = extract_http_error_text(response, "create bucket").await;
                debug!("Atlantic Response: POST /buckets - error (status: {}, error: {})", status, error_text);
                Err(AtlanticError::AtlanticService(status, error_text))
            }
        }
    }

    /// Close a bucket.
    /// No new child job can be added once the bucket is closed.
    /// We make sure that all the child jobs are completed before closing the bucket.
    /// It's closed in the Aggregator job.
    pub async fn close_bucket(
        &self,
        bucket_id: &str,
        atlantic_api_key: impl AsRef<str>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        debug!("Atlantic Request: POST /buckets/close - closing bucket (bucket_id: {})", bucket_id);

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
            .map_err(|e| AtlanticError::CloseBucketFailure {
                context: format!("bucket_id: {}", bucket_id),
                source: e,
            })?;

        let status = response.status();
        match status.is_success() {
            true => {
                let bucket_response = response.json().await.map_err(|e| AtlanticError::CloseBucketFailure {
                    context: format!("bucket_id: {}, parsing JSON response", bucket_id),
                    source: e,
                })?;
                debug!(
                    "Atlantic Response: POST /buckets/close - success (status: {}, bucket_id: {})",
                    status, bucket_id
                );
                Ok(bucket_response)
            }
            false => {
                let (error_text, status) = extract_http_error_text(response, "close bucket").await;
                debug!("Atlantic Response: POST /buckets/close - error (status: {}, error: {})", status, error_text);
                Err(AtlanticError::AtlanticService(status, error_text))
            }
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
        let job_size = Self::n_steps_to_job_size(job_info.n_steps);
        debug!(
            "Atlantic Request: POST /atlantic-query - submitting job (layout: {}, job_size: {}, network: {}, pie_file: {:?}, bucket_id: {:?}, bucket_job_index: {:?})",
            job_config.proof_layout,
            job_size,
            &job_config.network,
            job_info.pie_file,
            bucket_info.bucket_id,
            bucket_info.bucket_job_index
        );

        let mut request = self.proving_layer.add_proving_params(
            // NOTE: Removing layout from the query params as it is unnecessary now (as conveyed by the Atlantic team)
            self.client
                .request()
                .method(Method::POST)
                .path("atlantic-query")
                .query_param("apiKey", api_key.as_ref())
                .form_text("declaredJobSize", job_size)
                .form_text("result", &job_config.result.to_string())
                .form_text("network", job_config.network.as_ref())
                .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                .form_text("cairoVm", &job_config.cairo_vm.as_str())
                .form_text("externalId", &job_info.external_id)
                .form_file("pieFile", job_info.pie_file.as_ref(), "pie.zip", Some("application/zip"))?,

            ProvingParams { layout: job_config.proof_layout },
        );

        if let Some(ref bucket_id) = bucket_info.bucket_id {
            request = request.form_text("bucketId", bucket_id);
        }
        if let Some(bucket_job_index) = bucket_info.bucket_job_index {
            request = request.form_text("bucketJobIndex", &bucket_job_index.to_string());
        }

        let context = format!(
            "layout: {}, job_size: {}, network: {}, pie_file: {:?}, bucket_id: {:?}, bucket_job_index: {:?}",
            job_config.proof_layout,
            job_size,
            job_config.network,
            job_info.pie_file,
            bucket_info.bucket_id,
            bucket_info.bucket_job_index
        );

        let response =
            request.send().await.map_err(|e| AtlanticError::AddJobFailure { context: context.clone(), source: e })?;

        let status = response.status();
        match status.is_success() {
            true => {
                let job_response: AtlanticAddJobResponse = response.json().await.map_err(|e| {
                    AtlanticError::AddJobFailure { context: format!("{}, parsing JSON response", context), source: e }
                })?;
                debug!(
                    "Atlantic Response: POST /atlantic-query - success (status: {}, job_key: {})",
                    status, job_response.atlantic_query_id
                );
                Ok(job_response)
            }
            false => {
                let (error_text, status) = extract_http_error_text(response, "add job").await;
                debug!("Atlantic Response: POST /atlantic-query - error (status: {}, error: {})", status, error_text);
                Err(AtlanticError::AtlanticService(status, error_text))
            }
        }
    }

    /// Fetch the status of a job
    pub async fn get_job_status(&self, job_key: &str) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        debug!("Atlantic Request: GET /atlantic-query/{} - getting job status", job_key);

        self.retry_get("get_job_status", || async {
            let response =
                self.client.request().method(Method::GET).path("atlantic-query").path(job_key).send().await.map_err(
                    |e| AtlanticError::GetJobStatusFailure { context: format!("job_key: {}", job_key), source: e },
                )?;

            let status = response.status();
            if status.is_success() {
                let job_status: AtlanticGetStatusResponse =
                    response.json().await.map_err(|e| AtlanticError::GetJobStatusFailure {
                        context: format!("job_key: {}, parsing JSON response", job_key),
                        source: e,
                    })?;
                debug!(
                    "Atlantic Response: GET /atlantic-query/{} - success (status: {}, job_status: {:?})",
                    job_key, status, job_status.atlantic_query.status
                );
                Ok(job_status)
            } else {
                {
                    let (error_text, status) = extract_http_error_text(response, "get job status").await;
                    debug!(
                        "Atlantic Response: GET /atlantic-query/{} - error (status: {}, error: {})",
                        job_key, status, error_text
                    );
                    Err(AtlanticError::AtlanticService(status, error_text))
                }
            }
        })
        .await
    }

    /// Fetch proof from herodotus service.
    ///
    /// # Arguments
    /// task_id - the task id of the proof to get
    ///
    /// # Returns
    /// The proof as a string if the request is successful, otherwise an error is returned
    pub async fn get_proof_by_task_id(&self, task_id: &str) -> Result<String, AtlanticError> {
        // TODO: Update the code once a proper API is available for this
        let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);
        debug!("Atlantic Request: GET {} - getting proof for task_id: {}", proof_path, task_id);

        self.retry_get("get_proof_by_task_id", || async {
            let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);
            let client = reqwest::Client::new();
            let response = client.get(&proof_path).send().await.map_err(|e| AtlanticError::GetJobArtifactsFailure {
                context: format!("task_id: {}, url: {}", task_id, proof_path),
                source: e,
            })?;

            let status = response.status();
            if status.is_success() {
                let response_text = response.text().await.map_err(|e| AtlanticError::GetJobArtifactsFailure {
                    context: format!("task_id: {}, url: {}, reading response text", task_id, proof_path),
                    source: e,
                })?;
                let proof_size = response_text.len();
                debug!(
                    "Atlantic Response: GET {} - success (status: {}, task_id: {}, proof_size: {} bytes)",
                    proof_path, status, task_id, proof_size
                );
                Ok(response_text)
            } else {
                {
                    let (error_text, status) = extract_http_error_text(response, "get proof by task id").await;
                    debug!(
                        "Atlantic Response: GET {} - error (status: {}, task_id: {}, error: {})",
                        proof_path, status, task_id, error_text
                    );
                    Err(AtlanticError::AtlanticService(status, error_text))
                }
            }
        })
        .await
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
        let job_size = Self::n_steps_to_job_size(n_steps);
        let network = atlantic_network.as_ref();
        debug!(
            "Atlantic Request: POST /atlantic-query - submitting L2 query (job_size: {}, network: {}, program_hash: {}, proof_size: {} bytes)",
            job_size,
            network,
            program_hash,
            proof.len()
        );

        let context = format!(
            "job_size: {}, network: {}, program_hash: {}, proof_size: {} bytes",
            job_size,
            network,
            program_hash,
            proof.len()
        );

        let response = self
            .client
            .request()
            .method(Method::POST)
            .path("atlantic-query")
            .query_param("apiKey", atlantic_api_key.as_ref())
            .form_file_bytes("inputFile", proof.as_bytes().to_vec(), "proof.json", Some("application/json"))?
            .form_text("programHash", program_hash)
            .form_text("layout", LayoutName::recursive_with_poseidon.to_str())
            .form_text("declaredJobSize", job_size)
            .form_text("network", network)
            .form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
            .form_text("cairoVm", &AtlanticCairoVm::Python.as_str())
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
            .send()
            .await
            .map_err(|e| AtlanticError::AddJobFailure { context: context.clone(), source: e })?;

        let status = response.status();
        match status.is_success() {
            true => {
                let job_response: AtlanticAddJobResponse = response.json().await.map_err(|e| {
                    AtlanticError::AddJobFailure { context: format!("{}, parsing JSON response", context), source: e }
                })?;
                debug!(
                    "Atlantic Response: POST /atlantic-query - L2 query success (status: {}, job_key: {})",
                    status, job_response.atlantic_query_id
                );
                Ok(job_response)
            }
            false => {
                let (error_text, status) = extract_http_error_text(response, "submit L2 query").await;
                debug!(
                    "Atlantic Response: POST /atlantic-query - L2 query error (status: {}, error: {})",
                    status, error_text
                );
                Err(AtlanticError::AtlanticService(status, error_text))
            }
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
