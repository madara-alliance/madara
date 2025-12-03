use std::path::PathBuf;
use std::time::{Duration, Instant};

use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::Felt252;
use orchestrator_utils::http_client::extract_http_error_text;
use orchestrator_utils::http_client::{HttpClient, RequestBuilder};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::Method;
use tracing::{debug, info, warn};
use url::Url;

use crate::constants::{
    AGGREGATOR_FULL_OUTPUT, AGGREGATOR_USE_KZG_DA, ATLANTIC_PROOF_URL, RETRY_DELAY_SECONDS, RETRY_MAX_ATTEMPTS,
};
use crate::error::AtlanticError;
use crate::types::{
    AtlanticAddJobResponse, AtlanticAggregatorParams, AtlanticAggregatorVersion, AtlanticBucketResponse,
    AtlanticCairoVersion, AtlanticCairoVm, AtlanticCreateBucketRequest, AtlanticGetBucketResponse,
    AtlanticGetStatusResponse, AtlanticQueriesListResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;

/// API key header name used by Atlantic service
const API_KEY_HEADER: &str = "api-key";

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

    /// Generic retry mechanism for all Atlantic API calls (GET and POST)
    ///
    /// Retries network-level errors (timeouts, incomplete messages, connection issues)
    /// Does NOT retry API-level errors (4xx, 5xx with proper Atlantic responses)
    ///
    /// # Arguments
    /// * `operation_name` - Name of the operation for logging (e.g., "add_job", "get_bucket")
    /// * `context` - Additional context about the request (e.g., job params, bucket_id)
    /// * `data_size_bytes` - Size of request data for metrics
    /// * `f` - Async function to execute (the actual API call)
    /// * `metrics_extractor` - Function to extract response size for metrics
    async fn retry_request<F, Fut, T, M>(
        &self,
        operation_name: &str,
        context: &str,
        data_size_bytes: u64,
        mut f: F,
        mut metrics_extractor: M,
    ) -> Result<T, AtlanticError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, AtlanticError>>,
        M: FnMut(&T) -> u64,
    {
        let start_time = Instant::now();
        let mut last_error = None;
        let mut total_attempts = 0;
        let retry_count;

        for attempt in 1..=RETRY_MAX_ATTEMPTS {
            total_attempts = attempt;
            let attempt_start = Instant::now();

            match f().await {
                Ok(result) => {
                    let duration_s = start_time.elapsed().as_secs_f64();
                    let response_size_bytes = metrics_extractor(&result);
                    retry_count = if attempt > 1 { attempt - 1 } else { 0 };

                    // Emit metrics event
                    info!(
                        metric_type = "atlantic_api_call",
                        operation = operation_name,
                        duration_seconds = duration_s,
                        request_bytes = data_size_bytes,
                        response_bytes = response_size_bytes,
                        retry_count = retry_count,
                        success = true,
                        context = context,
                        "Atlantic API call completed successfully"
                    );

                    if attempt > 1 {
                        info!(
                            operation = operation_name,
                            context = context,
                            attempts = attempt,
                            "Atlantic API request succeeded after retry"
                        );
                    }
                    return Ok(result);
                }
                Err(err) => {
                    let attempt_duration = attempt_start.elapsed().as_millis();

                    // Check if error is retryable
                    let is_retryable = err.is_retryable();

                    if is_retryable && attempt < RETRY_MAX_ATTEMPTS {
                        warn!(
                            operation = operation_name,
                            context = context,
                            attempt = attempt,
                            max_attempts = RETRY_MAX_ATTEMPTS,
                            attempt_duration_ms = attempt_duration,
                            error = %err,
                            "Atlantic API request failed, retrying"
                        );
                        last_error = Some(err);
                        tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECONDS)).await;
                    } else {
                        // Non-retryable error or last attempt
                        let duration_s = start_time.elapsed().as_secs_f64();
                        retry_count = if attempt > 1 { attempt - 1 } else { 0 };

                        let error_type = err.error_type();

                        // Emit metrics event for failure
                        warn!(
                            metric_type = "atlantic_api_call",
                            operation = operation_name,
                            duration_seconds = duration_s,
                            request_bytes = data_size_bytes,
                            response_bytes = 0,
                            retry_count = retry_count,
                            success = false,
                            error_type = error_type,
                            context = context,
                            error = %err,
                            "Atlantic API call failed"
                        );

                        if !is_retryable {
                            debug!(
                                operation = operation_name,
                                context = context,
                                attempt = attempt,
                                error = %err,
                                "Atlantic API request failed with non-retryable error"
                            );
                        }
                        return Err(err);
                    }
                }
            }
        }

        // All retries exhausted
        let total_duration_s = start_time.elapsed().as_secs_f64();
        let final_error = last_error.expect("At least one attempt should have been made");
        retry_count = total_attempts - 1;
        let error_type = final_error.error_type();

        // Emit metrics event for exhausted retries
        warn!(
            metric_type = "atlantic_api_call",
            operation = operation_name,
            duration_seconds = total_duration_s,
            request_bytes = data_size_bytes,
            response_bytes = 0,
            retry_count = retry_count,
            success = false,
            error_type = error_type,
            context = context,
            error = %final_error,
            "Atlantic API request failed after all retries"
        );

        Err(final_error)
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
        let start_time = Instant::now();
        info!(
            operation = "get_artifacts",
            url = %artifact_path,
            "Starting Atlantic artifacts download"
        );

        let context = format!("url: {}", artifact_path);
        let artifact_path_clone = artifact_path.clone();

        let result = self
            .retry_request(
                "get_artifacts",
                &context,
                0,
                || {
                    let artifact_path = artifact_path_clone.clone();
                    async move {
                        debug!(
                            operation = "get_artifacts",
                            url = %artifact_path,
                            "Fetching artifact"
                        );

                        let client = reqwest::Client::new();
                        let response = client
                            .get(&artifact_path)
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("get_artifacts", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let response_bytes = response
                                .bytes()
                                .await
                                .map_err(|e| AtlanticError::from_reqwest_error("get_artifacts", e))?;
                            let artifact_size = response_bytes.len();

                            debug!(
                                operation = "get_artifacts",
                                url = %artifact_path,
                                status = %status,
                                artifact_size_bytes = artifact_size,
                                "Artifact download successful"
                            );
                            Ok(response_bytes.to_vec())
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "get artifacts").await;
                            debug!(
                                operation = "get_artifacts",
                                url = %artifact_path,
                                status = %status,
                                error = %error_text,
                                "Artifact download failed"
                            );
                            Err(AtlanticError::api_error("get_artifacts", status, error_text))
                        }
                    }
                },
                |response_bytes: &Vec<u8>| response_bytes.len() as u64,
            )
            .await?;

        info!(
            operation = "get_artifacts",
            url = %artifact_path,
            artifact_size_bytes = result.len(),
            duration_ms = start_time.elapsed().as_millis(),
            "Atlantic artifacts download completed"
        );

        Ok(result)
    }

    /// Fetch the details of a bucket from the Atlantic client
    pub async fn get_bucket(&self, bucket_id: &str) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        let start_time = Instant::now();
        let context = format!("bucket_id: {}", bucket_id);

        debug!(
            operation = "get_bucket",
            bucket_id = %bucket_id,
            "Getting bucket details"
        );

        let bucket_id = bucket_id.to_string();
        let result = self
            .retry_request(
                "get_bucket",
                &context,
                0,
                || {
                    let bucket_id = bucket_id.clone();
                    async move {
                        let response = self
                            .client
                            .request()
                            .method(Method::GET)
                            .path("buckets")
                            .path(&bucket_id)
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("get_bucket", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let bucket_response: AtlanticGetBucketResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("get_bucket", e.to_string()))?;

                            debug!(
                                operation = "get_bucket",
                                bucket_id = %bucket_id,
                                status = %status,
                                queries_count = bucket_response.queries.len(),
                                bucket_status = ?bucket_response.bucket.status,
                                "Bucket details retrieved successfully"
                            );
                            Ok(bucket_response)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "get bucket").await;
                            debug!(
                                operation = "get_bucket",
                                bucket_id = %bucket_id,
                                status = %status,
                                error = %error_text,
                                "Failed to get bucket details"
                            );
                            Err(AtlanticError::api_error("get_bucket", status, error_text))
                        }
                    }
                },
                |_: &AtlanticGetBucketResponse| 0,
            )
            .await?;

        info!(
            operation = "get_bucket",
            bucket_id = %bucket_id,
            duration_ms = start_time.elapsed().as_millis(),
            "Get bucket completed"
        );

        Ok(result)
    }

    /// Search Atlantic queries with optional filters
    /// This function searches through Atlantic queries using the provided search string
    /// and optional additional filters like limit, offset, network, status, and result.
    ///
    /// # Arguments
    /// * `atlantic_api_key` - API key for authentication (sent via header)
    /// * `search_string` - The search string to filter queries
    /// * `limit` - Optional limit for the number of results (default: None)
    /// * `network` - Optional network filter (default: None)
    ///
    /// # Returns
    /// Returns a list of Atlantic queries matching the search criteria and total count
    pub async fn search_atlantic_queries(
        &self,
        atlantic_api_key: impl AsRef<str>,
        search_string: impl AsRef<str>,
        limit: Option<u32>,
        network: Option<&str>,
    ) -> Result<AtlanticQueriesListResponse, AtlanticError> {
        let start_time = Instant::now();
        let search = search_string.as_ref().to_string();
        let context = format!("search: {}, limit: {:?}, network: {:?}", search, limit, network);

        debug!(
            operation = "search_atlantic_queries",
            search = %search,
            limit = ?limit,
            network = ?network,
            "Searching Atlantic queries"
        );

        let api_key = atlantic_api_key.as_ref().to_string();
        let network_str = network.map(|s| s.to_string());

        let result = self
            .retry_request(
                "search_atlantic_queries",
                &context,
                0,
                || {
                    let api_key = api_key.clone();
                    let search = search.clone();
                    let network = network_str.clone();

                    async move {
                        let mut request = self
                            .client
                            .request()
                            .method(Method::GET)
                            .header(
                                HeaderName::from_static(API_KEY_HEADER),
                                HeaderValue::from_str(&api_key).map_err(|e| AtlanticError::Other {
                                    operation: "search_atlantic_queries".to_string(),
                                    message: format!("Invalid API key header value: {}", e),
                                })?,
                            )
                            .path("atlantic-queries")
                            .query_param("search", &search);

                        // Add optional query parameters
                        if let Some(limit_val) = limit {
                            request = request.query_param("limit", &limit_val.to_string());
                        }
                        if let Some(ref network_val) = network {
                            request = request.query_param("network", network_val);
                        }

                        let response = request
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("search_atlantic_queries", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let queries_response: AtlanticQueriesListResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("search_atlantic_queries", e.to_string()))?;

                            debug!(
                                operation = "search_atlantic_queries",
                                status = %status,
                                search = %search,
                                total_results = queries_response.total,
                                "Search completed successfully"
                            );
                            Ok(queries_response)
                        } else {
                            let (error_text, status) =
                                extract_http_error_text(response, "search atlantic queries").await;
                            debug!(
                                operation = "search_atlantic_queries",
                                status = %status,
                                search = %search,
                                error = %error_text,
                                "Search failed"
                            );
                            Err(AtlanticError::api_error("search_atlantic_queries", status, error_text))
                        }
                    }
                },
                |_: &AtlanticQueriesListResponse| 0,
            )
            .await?;

        info!(
            operation = "search_atlantic_queries",
            search = %search,
            total_results = result.total,
            duration_ms = start_time.elapsed().as_millis(),
            "Search Atlantic queries completed"
        );

        Ok(result)
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
        let start_time = Instant::now();
        let context = format!("mock_proof: {}, chain_id_hex: {:?}", mock_proof, chain_id_hex);

        info!(
            operation = "create_bucket",
            mock_proof = mock_proof,
            chain_id_hex = ?chain_id_hex,
            "Starting bucket creation"
        );

        let api_key = atlantic_api_key.as_ref().to_string();
        let chain_id_clone = chain_id_hex.clone();

        let bucket_request = AtlanticCreateBucketRequest {
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
        };

        debug!(
            operation = "create_bucket",
            request = ?bucket_request,
            "Create bucket request details"
        );

        let result = self
            .retry_request(
                "create_bucket",
                &context,
                0,
                || {
                    let api_key = api_key.clone();
                    let bucket_request = AtlanticCreateBucketRequest {
                        external_id: None,
                        node_width: None,
                        aggregator_version: AtlanticAggregatorVersion::SnosAggregator0_13_3,
                        aggregator_params: AtlanticAggregatorParams {
                            use_kzg_da: AGGREGATOR_USE_KZG_DA,
                            full_output: AGGREGATOR_FULL_OUTPUT,
                            chain_id_hex: chain_id_clone.clone(),
                            fee_token_address,
                        },
                        mock_proof,
                    };

                    async move {
                        // TODO(prakhar,19/11/2025): Use the aggregator version calculated from Madara Version being passed through ENV
                        let response = self
                            .client
                            .request()
                            .method(Method::POST)
                            .header(ACCEPT, HeaderValue::from_static("application/json"))
                            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
                            .header(
                                HeaderName::from_static(API_KEY_HEADER),
                                HeaderValue::from_str(&api_key).map_err(|e| AtlanticError::Other {
                                    operation: "create_bucket".to_string(),
                                    message: format!("Invalid API key header value: {}", e),
                                })?,
                            )
                            .path("buckets")
                            .body(bucket_request)
                            .map_err(|e| AtlanticError::parse_error("create_bucket", e.to_string()))?
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("create_bucket", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let bucket_response: AtlanticBucketResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("create_bucket", e.to_string()))?;

                            debug!(
                                operation = "create_bucket",
                                status = %status,
                                bucket_id = %bucket_response.atlantic_bucket.id,
                                response = ?bucket_response,
                                "Bucket created successfully"
                            );
                            Ok(bucket_response)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "create bucket").await;
                            debug!(
                                operation = "create_bucket",
                                status = %status,
                                error = %error_text,
                                "Failed to create bucket"
                            );
                            Err(AtlanticError::api_error("create_bucket", status, error_text))
                        }
                    }
                },
                |_: &AtlanticBucketResponse| 0,
            )
            .await?;

        info!(
            operation = "create_bucket",
            bucket_id = %result.atlantic_bucket.id,
            duration_ms = start_time.elapsed().as_millis(),
            "Bucket creation completed"
        );

        Ok(result)
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
        let start_time = Instant::now();
        let context = format!("bucket_id: {}", bucket_id);

        info!(
            operation = "close_bucket",
            bucket_id = %bucket_id,
            "Starting bucket closure"
        );

        let bucket_id = bucket_id.to_string();
        let api_key = atlantic_api_key.as_ref().to_string();

        let result = self
            .retry_request(
                "close_bucket",
                &context,
                0,
                || {
                    let bucket_id = bucket_id.clone();
                    let api_key = api_key.clone();

                    async move {
                        debug!(
                            operation = "close_bucket",
                            bucket_id = %bucket_id,
                            "Sending close bucket request"
                        );

                        let response = self
                            .client
                            .request()
                            .method(Method::POST)
                            .header(ACCEPT, HeaderValue::from_static("application/json"))
                            .header(
                                HeaderName::from_static(API_KEY_HEADER),
                                HeaderValue::from_str(&api_key).map_err(|e| AtlanticError::Other {
                                    operation: "close_bucket".to_string(),
                                    message: format!("Invalid API key header value: {}", e),
                                })?,
                            )
                            .path("buckets")
                            .path("close")
                            .query_param("bucketId", &bucket_id)
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("close_bucket", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let bucket_response: AtlanticBucketResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("close_bucket", e.to_string()))?;

                            debug!(
                                operation = "close_bucket",
                                status = %status,
                                bucket_id = %bucket_id,
                                response = ?bucket_response,
                                "Bucket closed successfully"
                            );
                            Ok(bucket_response)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "close bucket").await;
                            debug!(
                                operation = "close_bucket",
                                status = %status,
                                bucket_id = %bucket_id,
                                error = %error_text,
                                "Failed to close bucket"
                            );
                            Err(AtlanticError::api_error("close_bucket", status, error_text))
                        }
                    }
                },
                |_: &AtlanticBucketResponse| 0,
            )
            .await?;

        info!(
            operation = "close_bucket",
            bucket_id = %bucket_id,
            duration_ms = start_time.elapsed().as_millis(),
            "Bucket closure completed"
        );

        Ok(result)
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
        let start_time = Instant::now();
        let job_size = Self::n_steps_to_job_size(job_info.n_steps);

        // Get file size for logging
        let file_size = std::fs::metadata(&job_info.pie_file).map(|m| m.len()).unwrap_or(0);

        info!(
            operation = "add_job",
            layout = %job_config.proof_layout,
            job_size = job_size,
            network = %job_config.network,
            pie_file_size_bytes = file_size,
            bucket_id = ?bucket_info.bucket_id,
            bucket_job_index = ?bucket_info.bucket_job_index,
            external_id = %job_info.external_id,
            "Starting job submission"
        );

        let context = format!(
            "layout: {}, job_size: {}, network: {}, pie_file_size: {} bytes, bucket_id: {:?}, bucket_job_index: {:?}, external_id: {}",
            job_config.proof_layout,
            job_size,
            job_config.network,
            file_size,
            bucket_info.bucket_id,
            bucket_info.bucket_job_index,
            job_info.external_id
        );

        let api_key_str = api_key.as_ref().to_string();
        let pie_file_path = job_info.pie_file.clone();
        let external_id = job_info.external_id.clone();
        let layout = job_config.proof_layout;
        let cairo_vm = job_config.cairo_vm.clone();
        let result_step = job_config.result.clone();
        let network = job_config.network.clone();
        let bucket_id = bucket_info.bucket_id.clone();
        let bucket_job_index = bucket_info.bucket_job_index;

        let result = self
            .retry_request(
                "add_job",
                &context,
                file_size,
                || {
                    let api_key = api_key_str.clone();
                    let pie_file = pie_file_path.clone();
                    let external_id = external_id.clone();
                    let cairo_vm = cairo_vm.clone();
                    let result_step = result_step.clone();
                    let network = network.clone();
                    let bucket_id = bucket_id.clone();

                    async move {
                        debug!(
                            operation = "add_job",
                            layout = %layout,
                            job_size = job_size,
                            network = %network,
                            pie_file = ?pie_file,
                            external_id = %external_id,
                            "Building add_job request"
                        );

                        let mut request = self.proving_layer.add_proving_params(
                            // NOTE: Removing layout from the query params as it is unnecessary now (as conveyed by the Atlantic team)
                            self.client
                                .request()
                                .method(Method::POST)
                                .header(
                                    HeaderName::from_static(API_KEY_HEADER),
                                    HeaderValue::from_str(&api_key).map_err(|e| AtlanticError::Other {
                                        operation: "add_job".to_string(),
                                        message: format!("Invalid API key header value: {}", e),
                                    })?,
                                )
                                .path("atlantic-query")
                                .form_text("declaredJobSize", job_size)
                                .form_text("result", &result_step.to_string())
                                .form_text("network", &network)
                                .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                                .form_text("cairoVm", &cairo_vm.as_str())
                                .form_text("externalId", &external_id)
                                .form_file("pieFile", pie_file.as_ref(), "pie.zip", Some("application/zip"))
                                .map_err(|e| AtlanticError::from_io_error("add_job", e))?,
                            ProvingParams { layout },
                        );

                        if let Some(ref bucket_id) = bucket_id {
                            request = request.form_text("bucketId", bucket_id);
                        }
                        if let Some(bucket_job_index) = bucket_job_index {
                            request = request.form_text("bucketJobIndex", &bucket_job_index.to_string());
                        }

                        debug!(
                            operation = "add_job",
                            layout = %layout,
                            job_size = job_size,
                            network = %network,
                            cairo_vm = ?cairo_vm,
                            result = ?result_step,
                            bucket_id = ?bucket_id,
                            bucket_job_index = ?bucket_job_index,
                            external_id = %external_id,
                            "Sending add_job request"
                        );

                        let response =
                            request.send().await.map_err(|e| AtlanticError::from_reqwest_error("add_job", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let job_response: AtlanticAddJobResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("add_job", e.to_string()))?;

                            debug!(
                                operation = "add_job",
                                status = %status,
                                job_id = %job_response.atlantic_query_id,
                                response = ?job_response,
                                "Job submitted successfully"
                            );
                            Ok(job_response)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "add job").await;
                            debug!(
                                operation = "add_job",
                                status = %status,
                                error = %error_text,
                                "Failed to submit job"
                            );
                            Err(AtlanticError::api_error("add_job", status, error_text))
                        }
                    }
                },
                |_: &AtlanticAddJobResponse| 0,
            )
            .await?;

        info!(
            operation = "add_job",
            job_id = %result.atlantic_query_id,
            pie_file_size_bytes = file_size,
            duration_ms = start_time.elapsed().as_millis(),
            "Job submission completed"
        );

        Ok(result)
    }

    /// Fetch the status of a job
    pub async fn get_job_status(&self, job_key: &str) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        let start_time = Instant::now();
        let context = format!("job_key: {}", job_key);

        debug!(
            operation = "get_job_status",
            job_key = %job_key,
            "Getting job status"
        );

        let job_key = job_key.to_string();
        let result = self
            .retry_request(
                "get_job_status",
                &context,
                0,
                || {
                    let job_key = job_key.clone();
                    async move {
                        let response = self
                            .client
                            .request()
                            .method(Method::GET)
                            .path("atlantic-query")
                            .path(&job_key)
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("get_job_status", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let job_status: AtlanticGetStatusResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("get_job_status", e.to_string()))?;

                            debug!(
                                operation = "get_job_status",
                                job_key = %job_key,
                                status = %status,
                                job_status = ?job_status.atlantic_query.status,
                                response = ?job_status,
                                "Job status retrieved successfully"
                            );
                            Ok(job_status)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "get job status").await;
                            debug!(
                                operation = "get_job_status",
                                job_key = %job_key,
                                status = %status,
                                error = %error_text,
                                "Failed to get job status"
                            );
                            Err(AtlanticError::api_error("get_job_status", status, error_text))
                        }
                    }
                },
                |_: &AtlanticGetStatusResponse| 0,
            )
            .await?;

        info!(
            operation = "get_job_status",
            job_key = %job_key,
            job_status = ?result.atlantic_query.status,
            duration_ms = start_time.elapsed().as_millis(),
            "Get job status completed"
        );

        Ok(result)
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
        let start_time = Instant::now();
        let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);

        info!(
            operation = "get_proof_by_task_id",
            task_id = %task_id,
            url = %proof_path,
            "Starting proof download"
        );

        let context = format!("task_id: {}, url: {}", task_id, proof_path);
        let task_id_clone = task_id.to_string();

        let result = self
            .retry_request(
                "get_proof_by_task_id",
                &context,
                0,
                || {
                    let proof_path = ATLANTIC_PROOF_URL.replace("{}", &task_id_clone);
                    let task_id = task_id_clone.clone();

                    async move {
                        debug!(
                            operation = "get_proof_by_task_id",
                            task_id = %task_id,
                            url = %proof_path,
                            "Fetching proof"
                        );

                        let client = reqwest::Client::new();
                        let response = client
                            .get(&proof_path)
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("get_proof_by_task_id", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let response_text = response
                                .text()
                                .await
                                .map_err(|e| AtlanticError::from_reqwest_error("get_proof_by_task_id", e))?;
                            let proof_size = response_text.len();

                            debug!(
                                operation = "get_proof_by_task_id",
                                task_id = %task_id,
                                url = %proof_path,
                                status = %status,
                                proof_size_bytes = proof_size,
                                "Proof download successful"
                            );
                            Ok(response_text)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "get proof by task id").await;
                            debug!(
                                operation = "get_proof_by_task_id",
                                task_id = %task_id,
                                url = %proof_path,
                                status = %status,
                                error = %error_text,
                                "Proof download failed"
                            );
                            Err(AtlanticError::api_error("get_proof_by_task_id", status, error_text))
                        }
                    }
                },
                |response: &String| response.len() as u64,
            )
            .await?;

        info!(
            operation = "get_proof_by_task_id",
            task_id = %task_id,
            proof_size_bytes = result.len(),
            duration_ms = start_time.elapsed().as_millis(),
            "Proof download completed"
        );

        Ok(result)
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
        let start_time = Instant::now();
        let job_size = Self::n_steps_to_job_size(n_steps);
        let network = atlantic_network.as_ref();
        let proof_size = proof.len();

        info!(
            operation = "submit_l2_query",
            job_size = job_size,
            network = %network,
            program_hash = %program_hash,
            proof_size_bytes = proof_size,
            "Starting L2 query submission"
        );

        let context = format!(
            "job_size: {}, network: {}, program_hash: {}, proof_size: {} bytes",
            job_size, network, program_hash, proof_size
        );

        let api_key = atlantic_api_key.to_string();
        let network_str = network.to_string();
        let program_hash_str = program_hash.to_string();
        let proof_str = proof.to_string();

        let result = self
            .retry_request(
                "submit_l2_query",
                &context,
                proof_size as u64,
                || {
                    let api_key = api_key.clone();
                    let network = network_str.clone();
                    let program_hash = program_hash_str.clone();
                    let proof = proof_str.clone();

                    async move {
                        debug!(
                            operation = "submit_l2_query",
                            job_size = job_size,
                            network = %network,
                            program_hash = %program_hash,
                            proof_size_bytes = proof.len(),
                            layout = %LayoutName::recursive_with_poseidon.to_str(),
                            cairo_vm = "python",
                            "Sending L2 query request"
                        );

                        let response = self
                            .client
                            .request()
                            .method(Method::POST)
                            .header(
                                HeaderName::from_static(API_KEY_HEADER),
                                HeaderValue::from_str(&api_key).map_err(|e| AtlanticError::Other {
                                    operation: "submit_l2_query".to_string(),
                                    message: format!("Invalid API key header value: {}", e),
                                })?,
                            )
                            .path("atlantic-query")
                            .form_file_bytes(
                                "inputFile",
                                proof.as_bytes().to_vec(),
                                "proof.json",
                                Some("application/json"),
                            )
                            .map_err(|e| AtlanticError::from_io_error("submit_l2_query", e))?
                            .form_text("programHash", &program_hash)
                            .form_text("layout", LayoutName::recursive_with_poseidon.to_str())
                            .form_text("declaredJobSize", job_size)
                            .form_text("network", &network)
                            .form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
                            .form_text("cairoVm", &AtlanticCairoVm::Python.as_str())
                            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
                            .send()
                            .await
                            .map_err(|e| AtlanticError::from_reqwest_error("submit_l2_query", e))?;

                        let status = response.status();
                        if status.is_success() {
                            let job_response: AtlanticAddJobResponse = response
                                .json()
                                .await
                                .map_err(|e| AtlanticError::parse_error("submit_l2_query", e.to_string()))?;

                            debug!(
                                operation = "submit_l2_query",
                                status = %status,
                                job_id = %job_response.atlantic_query_id,
                                response = ?job_response,
                                "L2 query submitted successfully"
                            );
                            Ok(job_response)
                        } else {
                            let (error_text, status) = extract_http_error_text(response, "submit L2 query").await;
                            debug!(
                                operation = "submit_l2_query",
                                status = %status,
                                error = %error_text,
                                "Failed to submit L2 query"
                            );
                            Err(AtlanticError::api_error("submit_l2_query", status, error_text))
                        }
                    }
                },
                |_: &AtlanticAddJobResponse| 0,
            )
            .await?;

        info!(
            operation = "submit_l2_query",
            job_id = %result.atlantic_query_id,
            proof_size_bytes = proof_size,
            duration_ms = start_time.elapsed().as_millis(),
            "L2 query submission completed"
        );

        Ok(result)
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
