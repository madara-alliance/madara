use std::path::PathBuf;
use std::time::{Duration, Instant};

use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::Felt252;
use orchestrator_utils::http_client::HttpClient;
use tracing::{debug, info, warn};
use url::Url;

use crate::api::AtlanticApiOperations;
use crate::constants::{ATLANTIC_PROOF_URL, RETRY_DELAY_SECONDS, RETRY_MAX_ATTEMPTS};
use crate::error::AtlanticError;
use crate::metrics::ATLANTIC_METRICS;
use crate::proving::{create_proving_layer, ProvingLayer, ProvingParams};
use crate::transport::ApiKeyAuth;
use crate::types::{
    AtlanticAddJobResponse, AtlanticBucketResponse, AtlanticCairoVm, AtlanticGetBucketResponse,
    AtlanticGetStatusResponse, AtlanticQueryStep,
};
use crate::AtlanticValidatedArgs;

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

/// Atlantic API client
///
/// Provides high-level methods for interacting with the Atlantic prover service.
/// Handles retry logic, metrics recording, and delegates to the API layer for
/// request building and response parsing.
///
/// # Architecture
///
/// The client uses a three-layer architecture:
/// - **Client Layer** (this file): Retry orchestration, metrics, public API
/// - **API Layer** (`api.rs`): Request building, response parsing
/// - **Transport Layer** (`transport.rs`): Authentication, error classification
pub struct AtlanticClient {
    client: HttpClient,
    proving_layer: Box<dyn ProvingLayer>,
    /// Shared HTTP client for external artifact downloads (with connection pooling)
    http_client: reqwest::Client,
}

impl AtlanticClient {
    /// Creates a new Atlantic client with the given configuration
    pub fn new_with_args(url: Url, atlantic_params: &AtlanticValidatedArgs) -> Result<Self, AtlanticError> {
        let mock_fact_hash = atlantic_params.atlantic_mock_fact_hash.clone();
        let client = HttpClient::builder(url.as_str())
            .expect("Failed to create HTTP client builder")
            .default_form_data("mockFactHash", &mock_fact_hash)
            .build()
            .expect("Failed to build HTTP client");

        let proving_layer = create_proving_layer(&atlantic_params.atlantic_settlement_layer)?;

        // Create a shared HTTP client for external artifact downloads
        // This client reuses connections and is more efficient than creating new clients per request
        let http_client = reqwest::Client::new();

        Ok(Self { client, proving_layer, http_client })
    }

    /// Generic retry mechanism for all Atlantic API calls
    ///
    /// Retries network-level errors (timeouts, incomplete messages, connection issues).
    /// Does NOT retry API-level errors (4xx, 5xx with proper Atlantic responses).
    ///
    /// # Arguments
    /// * `operation_name` - Name of the operation for logging and metrics
    /// * `context` - Additional context about the request
    /// * `f` - Async function to execute (the actual API call)
    async fn retry_request<F, Fut, T>(&self, operation_name: &str, context: &str, mut f: F) -> Result<T, AtlanticError>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, AtlanticError>>,
    {
        let start_time = Instant::now();
        let mut last_error = None;

        for attempt in 1..=RETRY_MAX_ATTEMPTS {
            let attempt_start = Instant::now();

            match f().await {
                Ok(result) => {
                    let duration_s = start_time.elapsed().as_secs_f64();
                    let retry_count = attempt.saturating_sub(1);

                    // Record OTEL metrics
                    ATLANTIC_METRICS.record_success(operation_name, duration_s, retry_count);

                    // Only log if retries were needed (interesting case)
                    if attempt > 1 {
                        info!(
                            operation = operation_name,
                            attempts = attempt,
                            duration_ms = (duration_s * 1000.0) as u64,
                            "Atlantic API succeeded after retry"
                        );
                    }

                    debug!(
                        operation = operation_name,
                        duration_ms = (duration_s * 1000.0) as u64,
                        retry_count = retry_count,
                        "Atlantic API call completed"
                    );
                    return Ok(result);
                }
                Err(err) => {
                    let attempt_duration = attempt_start.elapsed();

                    // Check if error is retryable
                    let is_retryable = err.is_retryable();

                    if is_retryable && attempt < RETRY_MAX_ATTEMPTS {
                        debug!(
                            operation = operation_name,
                            attempt = attempt,
                            max_attempts = RETRY_MAX_ATTEMPTS,
                            duration_ms = attempt_duration.as_millis(),
                            error = %err,
                            "Atlantic API retry attempt"
                        );
                        warn!(
                            operation = operation_name,
                            attempt = attempt,
                            max_attempts = RETRY_MAX_ATTEMPTS,
                            error = %err,
                            "Atlantic API failed, retrying"
                        );
                        last_error = Some(err);
                        tokio::time::sleep(Duration::from_secs(RETRY_DELAY_SECONDS)).await;
                    } else {
                        // Non-retryable error or last attempt
                        let duration_s = start_time.elapsed().as_secs_f64();
                        let retry_count = attempt.saturating_sub(1);
                        let error_type = err.error_type();

                        // Record OTEL metrics for failure
                        ATLANTIC_METRICS.record_failure(operation_name, duration_s, error_type, retry_count);

                        // Log failure with error details
                        warn!(
                            operation = operation_name,
                            error_type = error_type,
                            retry_count = retry_count,
                            error = %err,
                            "Atlantic API call failed"
                        );
                        return Err(err);
                    }
                }
            }
        }

        // All retries exhausted - loop guarantees at least one attempt was made
        let total_duration_s = start_time.elapsed().as_secs_f64();
        let final_error = last_error.expect("At least one attempt should have been made");
        let retry_count = RETRY_MAX_ATTEMPTS - 1;
        let error_type = final_error.error_type();

        // Record OTEL metrics for exhausted retries
        ATLANTIC_METRICS.record_failure(operation_name, total_duration_s, error_type, retry_count);

        // Emit metrics event for exhausted retries
        warn!(
            metric_type = "atlantic_api_call",
            operation = operation_name,
            duration_seconds = total_duration_s,
            retry_count = retry_count,
            success = false,
            error_type = error_type,
            context = context,
            error = %final_error,
            "Atlantic API request failed after all retries"
        );

        Err(final_error)
    }

    // ==================== PUBLIC API METHODS ====================

    /// Fetch an artifact from the given path
    ///
    /// Downloads artifacts like proofs, SNOS output, Cairo PIE files from storage.
    ///
    /// # Arguments
    /// * `artifact_path` - Full URL to the artifact
    ///
    /// # Returns
    /// The artifact as a byte array
    pub async fn get_artifacts(&self, artifact_path: String) -> Result<Vec<u8>, AtlanticError> {
        let context = format!("url: {}", artifact_path);
        let artifact_path_clone = artifact_path.clone();

        self.retry_request("get_artifacts", &context, || {
            let artifact_path = artifact_path_clone.clone();
            let client = &self.http_client;
            async move {
                debug!(
                    operation = "get_artifacts",
                    url = %artifact_path,
                    "Fetching artifact"
                );

                // Use shared HTTP client for external URLs (enables connection pooling)
                let response = client
                    .get(&artifact_path)
                    .send()
                    .await
                    .map_err(|e| AtlanticError::from_reqwest_error("get_artifacts", e))?;

                AtlanticApiOperations::parse_artifacts_response(response, &artifact_path).await
            }
        })
        .await
    }

    /// Fetch the details of a bucket
    ///
    /// # Arguments
    /// * `bucket_id` - ID of the bucket to fetch
    ///
    /// # Returns
    /// Bucket details including status and associated queries
    pub async fn get_bucket(&self, bucket_id: &str) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        let context = format!("bucket_id: {}", bucket_id);
        let bucket_id_owned = bucket_id.to_string();

        debug!(
            operation = "get_bucket",
            bucket_id = %bucket_id,
            "Getting bucket details"
        );

        self.retry_request("get_bucket", &context, || {
            let bucket_id = bucket_id_owned.clone();
            async move {
                // Build request using API layer
                let request = AtlanticApiOperations::build_get_bucket_request(self.client.request(), &bucket_id);

                // Send request
                let response = request.send().await.map_err(|e| AtlanticError::from_reqwest_error("get_bucket", e))?;

                // Parse response using API layer
                AtlanticApiOperations::parse_get_bucket_response(response, &bucket_id).await
            }
        })
        .await
    }

    /// Create a new bucket for Applicative Recursion
    ///
    /// Creates an empty bucket that can have child jobs added to it.
    /// The bucket_id returned is used to add jobs via `add_job`.
    ///
    /// # Arguments
    /// * `atlantic_api_key` - API key for authentication
    /// * `mock_proof` - Whether to use mock proofs
    /// * `chain_id_hex` - Optional chain ID in hex format
    /// * `fee_token_address` - Optional fee token address
    ///
    /// # Returns
    /// Bucket response containing the new bucket ID
    pub async fn create_bucket(
        &self,
        atlantic_api_key: impl AsRef<str>,
        mock_proof: bool,
        chain_id_hex: Option<String>,
        fee_token_address: Option<Felt252>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        let context = format!("mock_proof: {}, chain_id_hex: {:?}", mock_proof, chain_id_hex);

        info!(
            operation = "create_bucket",
            mock_proof = mock_proof,
            chain_id_hex = ?chain_id_hex,
            "Starting bucket creation"
        );

        // Validate API key once using transport layer
        let auth = ApiKeyAuth::new(atlantic_api_key)?;
        let chain_id_clone = chain_id_hex.clone();

        let result = self
            .retry_request("create_bucket", &context, || {
                let auth = auth.clone();
                let chain_id = chain_id_clone.clone();

                async move {
                    // Build request using API layer
                    let request = AtlanticApiOperations::build_create_bucket_request(
                        self.client.request(),
                        &auth,
                        mock_proof,
                        chain_id,
                        fee_token_address,
                    )?;

                    // Send request
                    let response =
                        request.send().await.map_err(|e| AtlanticError::from_reqwest_error("create_bucket", e))?;

                    // Parse response using API layer
                    AtlanticApiOperations::parse_bucket_response(response, "create_bucket").await
                }
            })
            .await?;

        info!(
            operation = "create_bucket",
            bucket_id = %result.atlantic_bucket.id,
            "Bucket created"
        );

        Ok(result)
    }

    /// Close a bucket
    ///
    /// No new child jobs can be added once the bucket is closed.
    /// Ensure all child jobs are completed before closing.
    ///
    /// # Arguments
    /// * `bucket_id` - ID of the bucket to close
    /// * `atlantic_api_key` - API key for authentication
    pub async fn close_bucket(
        &self,
        bucket_id: &str,
        atlantic_api_key: impl AsRef<str>,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        let context = format!("bucket_id: {}", bucket_id);

        // Validate API key once using transport layer
        let auth = ApiKeyAuth::new(atlantic_api_key)?;
        let bucket_id_owned = bucket_id.to_string();

        let result = self
            .retry_request("close_bucket", &context, || {
                let bucket_id = bucket_id_owned.clone();
                let auth = auth.clone();

                async move {
                    debug!(
                        operation = "close_bucket",
                        bucket_id = %bucket_id,
                        "Sending close bucket request"
                    );

                    // Build request using API layer
                    let request =
                        AtlanticApiOperations::build_close_bucket_request(self.client.request(), &auth, &bucket_id);

                    // Send request
                    let response =
                        request.send().await.map_err(|e| AtlanticError::from_reqwest_error("close_bucket", e))?;

                    // Parse response using API layer
                    AtlanticApiOperations::parse_bucket_response(response, "close_bucket").await
                }
            })
            .await?;

        info!(
            operation = "close_bucket",
            bucket_id = %bucket_id,
            "Bucket closed"
        );

        Ok(result)
    }

    /// Submit a job to the prover
    ///
    /// Submits a Cairo PIE file for proving. Can optionally be added to a bucket
    /// for aggregated proving.
    ///
    /// # Arguments
    /// * `job_info` - Job information including PIE file path and steps
    /// * `job_config` - Job configuration including layout, VM, and network
    /// * `bucket_info` - Optional bucket association
    /// * `api_key` - API key for authentication
    pub async fn add_job(
        &self,
        job_info: AtlanticJobInfo,
        job_config: AtlanticJobConfig,
        bucket_info: AtlanticBucketInfo,
        api_key: impl AsRef<str>,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let job_size = AtlanticApiOperations::n_steps_to_job_size(job_info.n_steps);

        let context = format!(
            "layout: {}, job_size: {}, network: {}, bucket_id: {:?}, bucket_job_index: {:?}",
            job_config.proof_layout, job_size, job_config.network, bucket_info.bucket_id, bucket_info.bucket_job_index
        );

        // Validate API key once using transport layer
        let auth = ApiKeyAuth::new(api_key)?;
        let pie_file_path = job_info.pie_file.clone();
        let layout = job_config.proof_layout;
        let cairo_vm = job_config.cairo_vm.clone();
        let result_step = job_config.result.clone();
        let network = job_config.network.clone();
        let bucket_id = bucket_info.bucket_id.clone();
        let bucket_job_index = bucket_info.bucket_job_index;

        let result = self
            .retry_request("add_job", &context, || {
                let auth = auth.clone();
                let pie_file = pie_file_path.clone();
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
                        "Building add_job request"
                    );

                    // Build base request using API layer
                    let request = AtlanticApiOperations::build_add_job_request(
                        self.client.request(),
                        &auth,
                        &pie_file,
                        job_size,
                        &result_step,
                        &network,
                        &cairo_vm,
                        bucket_id.as_deref(),
                        bucket_job_index,
                    )?;

                    // Add proving layer specific params
                    let request = self.proving_layer.add_proving_params(request, ProvingParams { layout });

                    debug!(
                        operation = "add_job",
                        layout = %layout,
                        job_size = job_size,
                        network = %network,
                        cairo_vm = ?cairo_vm,
                        result = ?result_step,
                        bucket_id = ?bucket_id,
                        bucket_job_index = ?bucket_job_index,
                        "Sending add_job request"
                    );

                    // Send request
                    let response = request.send().await.map_err(|e| AtlanticError::from_reqwest_error("add_job", e))?;

                    // Parse response using API layer
                    AtlanticApiOperations::parse_job_response(response, "add_job").await
                }
            })
            .await?;

        info!(
            operation = "add_job",
            job_id = %result.atlantic_query_id,
            "Job submitted"
        );

        Ok(result)
    }

    /// Fetch the status of a job
    ///
    /// # Arguments
    /// * `job_key` - ID of the job to check
    ///
    /// # Returns
    /// Job status including current step and any error information
    pub async fn get_job_status(&self, job_key: &str) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        let context = format!("job_key: {}", job_key);
        let job_key_owned = job_key.to_string();

        debug!(
            operation = "get_job_status",
            job_key = %job_key,
            "Getting job status"
        );

        self.retry_request("get_job_status", &context, || {
            let job_key = job_key_owned.clone();
            async move {
                // Build request using API layer
                let request = AtlanticApiOperations::build_get_job_status_request(self.client.request(), &job_key);

                // Send request
                let response =
                    request.send().await.map_err(|e| AtlanticError::from_reqwest_error("get_job_status", e))?;

                // Parse response using API layer
                AtlanticApiOperations::parse_get_status_response(response, &job_key).await
            }
        })
        .await
    }

    /// Fetch proof by task ID
    ///
    /// Downloads the proof file for a completed job.
    ///
    /// # Arguments
    /// * `task_id` - ID of the task to get proof for
    ///
    /// # Returns
    /// The proof as a JSON string
    pub async fn get_proof_by_task_id(&self, task_id: &str) -> Result<String, AtlanticError> {
        let proof_path = ATLANTIC_PROOF_URL.replace("{}", task_id);
        let context = format!("task_id: {}, url: {}", task_id, proof_path);
        let task_id_owned = task_id.to_string();

        self.retry_request("get_proof_by_task_id", &context, || {
            let proof_path = ATLANTIC_PROOF_URL.replace("{}", &task_id_owned);
            let task_id = task_id_owned.clone();
            let client = &self.http_client;

            async move {
                debug!(
                    operation = "get_proof_by_task_id",
                    task_id = %task_id,
                    url = %proof_path,
                    "Fetching proof"
                );

                // Use shared HTTP client for external URLs (enables connection pooling)
                let response = client
                    .get(&proof_path)
                    .send()
                    .await
                    .map_err(|e| AtlanticError::from_reqwest_error("get_proof_by_task_id", e))?;

                // Parse response using API layer
                AtlanticApiOperations::parse_proof_response(response, &task_id, &proof_path).await
            }
        })
        .await
    }

    /// Submit L2 query for proof verification
    ///
    /// Submits a proof for verification on L2.
    ///
    /// # Arguments
    /// * `proof` - The proof JSON string
    /// * `n_steps` - Optional number of steps
    /// * `atlantic_network` - Network identifier
    /// * `atlantic_api_key` - API key for authentication
    /// * `program_hash` - Program hash for verification
    pub async fn submit_l2_query(
        &self,
        proof: &str,
        n_steps: Option<usize>,
        atlantic_network: impl AsRef<str>,
        atlantic_api_key: &str,
        program_hash: &str,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let job_size = AtlanticApiOperations::n_steps_to_job_size(n_steps);
        let network = atlantic_network.as_ref();
        let proof_size = proof.len();

        let context = format!(
            "job_size: {}, network: {}, program_hash: {}, proof_size: {} bytes",
            job_size, network, program_hash, proof_size
        );

        // Validate API key once using transport layer
        let auth = ApiKeyAuth::new(atlantic_api_key)?;
        let network_str = network.to_string();
        let program_hash_str = program_hash.to_string();
        let proof_str = proof.to_string();

        let result = self
            .retry_request("submit_l2_query", &context, || {
                let auth = auth.clone();
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

                    // Build request using API layer
                    let request = AtlanticApiOperations::build_submit_l2_query_request(
                        self.client.request(),
                        &auth,
                        &proof,
                        job_size,
                        &network,
                        &program_hash,
                    )?;

                    // Send request
                    let response =
                        request.send().await.map_err(|e| AtlanticError::from_reqwest_error("submit_l2_query", e))?;

                    // Parse response using API layer
                    AtlanticApiOperations::parse_job_response(response, "submit_l2_query").await
                }
            })
            .await?;

        info!(
            operation = "submit_l2_query",
            job_id = %result.atlantic_query_id,
            "L2 query submitted"
        );

        Ok(result)
    }
}
