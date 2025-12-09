//! Atlantic API operations layer
//!
//! This module handles request building and response parsing for all Atlantic API endpoints.
//! It encapsulates the API structure (paths, form fields, JSON shapes) while remaining
//! agnostic to HTTP transport details, retry logic, and metrics collection.
//!
//! # Architecture
//!
//! This is the middle layer in the three-layer architecture:
//!
//! ```text
//! ┌─────────────────────────────┐
//! │      Client Layer           │  ← Retry logic, metrics, public API
//! ├─────────────────────────────┤
//! │     API Layer (here)        │  ← Request building, response parsing
//! ├─────────────────────────────┤
//! │    Transport Layer          │  ← Authentication, error classification
//! └─────────────────────────────┘
//! ```
//!
//! # Design Principles
//!
//! - **`build_*` methods**: Construct HTTP requests from business parameters.
//!   Returns a `RequestBuilder` ready to send, but does NOT send it.
//!
//! - **`parse_*` methods**: Deserialize successful responses or map errors.
//!   Handles both success and error cases with appropriate error types.
//!
//! - **No side effects**: This layer doesn't send requests, record metrics,
//!   or perform retries. It purely transforms data.
//!
//! - **Debug logging**: Logs request/response details at DEBUG level for
//!   troubleshooting without cluttering production logs.
//!
//! # Supported Operations
//!
//! | Operation | Build Method | Parse Method |
//! |-----------|--------------|--------------|
//! | Create bucket | `build_create_bucket_request` | `parse_bucket_response` |
//! | Close bucket | `build_close_bucket_request` | `parse_bucket_response` |
//! | Add job | `build_add_job_request` | `parse_job_response` |
//! | Get bucket | `build_get_bucket_request` | `parse_get_bucket_response` |
//! | Get job status | `build_get_job_status_request` | `parse_get_status_response` |
//! | Submit L2 query | `build_submit_l2_query_request` | `parse_job_response` |
//! | Get artifacts | (direct HTTP) | `parse_artifacts_response` |
//! | Get proof | (direct HTTP) | `parse_proof_response` |
//!
//! # Example
//!
//! ```ignore
//! // Build request (API layer)
//! let request = AtlanticApiOperations::build_create_bucket_request(
//!     client.request(),
//!     &auth,
//!     mock_proof,
//!     chain_id_hex,
//!     fee_token_address,
//! )?;
//!
//! // Send request (handled by client layer)
//! let response = request.send().await?;
//!
//! // Parse response (API layer)
//! let result = AtlanticApiOperations::parse_bucket_response(response, "create_bucket").await?;
//! ```

use std::path::Path;

use cairo_vm::types::layout_name::LayoutName;
use cairo_vm::Felt252;
use orchestrator_utils::http_client::{extract_http_error_text, RequestBuilder};
use reqwest::header::{HeaderValue, ACCEPT, CONTENT_TYPE};
use reqwest::{Method, Response};
use tracing::debug;

use crate::constants::{AGGREGATOR_FULL_OUTPUT, AGGREGATOR_USE_KZG_DA};
use crate::error::AtlanticError;
use crate::transport::ApiKeyAuth;
use crate::types::{
    AtlanticAddJobResponse, AtlanticAggregatorParams, AtlanticAggregatorVersion, AtlanticBucketResponse,
    AtlanticCairoVersion, AtlanticCairoVm, AtlanticCreateBucketRequest, AtlanticGetBucketResponse,
    AtlanticGetStatusResponse, AtlanticQueryStep,
};

/// Atlantic API operations - request building and response parsing
///
/// All methods are stateless and operate on the provided parameters.
/// This struct provides a clean separation between API-level concerns
/// and transport/retry concerns.
///
/// # Example
///
/// ```ignore
/// // Build request (API layer)
/// let request = AtlanticApiOperations::build_create_bucket_request(
///     client.request(),
///     &auth,
///     mock_proof,
///     chain_id_hex,
///     fee_token_address,
/// )?;
///
/// // Send request (transport layer, handled by caller)
/// let response = request.send().await?;
///
/// // Parse response (API layer)
/// let result = AtlanticApiOperations::parse_bucket_response(response, "create_bucket").await?;
/// ```
pub struct AtlanticApiOperations;

impl AtlanticApiOperations {
    // ==================== CREATE BUCKET ====================

    /// Builds a create bucket request
    ///
    /// # Arguments
    /// * `builder` - Base request builder from HTTP client
    /// * `auth` - API key authentication
    /// * `mock_proof` - Whether to use mock proofs
    /// * `chain_id_hex` - Optional chain ID in hex format
    /// * `fee_token_address` - Optional fee token address
    ///
    /// # Returns
    /// Configured request builder ready to send
    pub fn build_create_bucket_request<'a>(
        builder: RequestBuilder<'a>,
        auth: &ApiKeyAuth,
        mock_proof: bool,
        chain_id_hex: Option<String>,
        fee_token_address: Option<Felt252>,
    ) -> Result<RequestBuilder<'a>, AtlanticError> {
        let bucket_request = AtlanticCreateBucketRequest {
            external_id: None,
            node_width: None,
            aggregator_version: AtlanticAggregatorVersion::SnosAggregator0_13_3,
            aggregator_params: AtlanticAggregatorParams {
                use_kzg_da: AGGREGATOR_USE_KZG_DA,
                full_output: AGGREGATOR_FULL_OUTPUT,
                chain_id_hex,
                fee_token_address,
            },
            mock_proof,
        };

        debug!(
            operation = "create_bucket",
            request = ?bucket_request,
            "Building create bucket request"
        );

        builder
            .method(Method::POST)
            .header(ApiKeyAuth::header_name(), auth.header_value())
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .path("buckets")
            .body(bucket_request)
            .map_err(|e| AtlanticError::parse_error("create_bucket", e.to_string()))
    }

    /// Parses create/close bucket response
    ///
    /// Handles both success and error cases, extracting appropriate error messages.
    pub async fn parse_bucket_response(
        response: Response,
        operation: &str,
    ) -> Result<AtlanticBucketResponse, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let bucket_response: AtlanticBucketResponse =
                response.json().await.map_err(|e| AtlanticError::parse_error(operation, e.to_string()))?;

            debug!(
                operation = operation,
                status = %status,
                bucket_id = %bucket_response.atlantic_bucket.id,
                "Bucket response parsed successfully"
            );

            Ok(bucket_response)
        } else {
            let (error_text, status) = extract_http_error_text(response, operation).await;
            debug!(
                operation = operation,
                status = %status,
                error = %error_text,
                "Bucket request failed"
            );
            Err(AtlanticError::from_http_error_response(operation, status, error_text))
        }
    }

    // ==================== CLOSE BUCKET ====================

    /// Builds a close bucket request
    pub fn build_close_bucket_request<'a>(
        builder: RequestBuilder<'a>,
        auth: &ApiKeyAuth,
        bucket_id: &str,
    ) -> RequestBuilder<'a> {
        debug!(
            operation = "close_bucket",
            bucket_id = %bucket_id,
            "Building close bucket request"
        );

        builder
            .method(Method::POST)
            .header(ApiKeyAuth::header_name(), auth.header_value())
            .header(ACCEPT, HeaderValue::from_static("application/json"))
            .path("buckets")
            .path("close")
            .query_param("bucketId", bucket_id)
    }

    // ==================== ADD JOB ====================

    /// Builds an add job request with file upload
    ///
    /// This is a complex multipart form request that includes:
    /// - Cairo PIE file upload
    /// - Job configuration parameters
    /// - Optional bucket association
    ///
    /// # Arguments
    /// * `builder` - Base request builder
    /// * `auth` - API key authentication
    /// * `pie_file` - Path to the Cairo PIE file
    /// * `job_size` - Declared job size (S, M, L)
    /// * `result` - Query step result type
    /// * `network` - Network identifier
    /// * `cairo_vm` - Cairo VM type (Rust/Python)
    /// * `bucket_id` - Optional bucket to add job to
    /// * `bucket_job_index` - Optional job index within bucket
    #[allow(clippy::too_many_arguments)]
    pub fn build_add_job_request<'a>(
        builder: RequestBuilder<'a>,
        auth: &ApiKeyAuth,
        pie_file: &Path,
        job_size: &'static str,
        result: &AtlanticQueryStep,
        network: &str,
        cairo_vm: &AtlanticCairoVm,
        bucket_id: Option<&str>,
        bucket_job_index: Option<u64>,
    ) -> Result<RequestBuilder<'a>, AtlanticError> {
        debug!(
            operation = "add_job",
            job_size = job_size,
            network = %network,
            cairo_vm = ?cairo_vm,
            result = ?result,
            bucket_id = ?bucket_id,
            bucket_job_index = ?bucket_job_index,
            pie_file = ?pie_file,
            "Building add job request"
        );

        let mut request = builder
            .method(Method::POST)
            .header(ApiKeyAuth::header_name(), auth.header_value())
            .path("atlantic-query")
            .form_text("declaredJobSize", job_size)
            .form_text("result", &result.to_string())
            .form_text("network", network)
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str())
            .form_text("cairoVm", &cairo_vm.as_str())
            .form_file("pieFile", pie_file, "pie.zip", Some("application/zip"))
            .map_err(|e| AtlanticError::from_io_error("add_job", e))?;

        if let Some(bucket_id) = bucket_id {
            request = request.form_text("bucketId", bucket_id);
        }
        if let Some(bucket_job_index) = bucket_job_index {
            request = request.form_text("bucketJobIndex", &bucket_job_index.to_string());
        }

        Ok(request)
    }

    /// Adds proving layer specific parameters to a request
    ///
    /// This is called by the ProvingLayer trait implementations
    /// to add network-specific parameters.
    pub fn add_proving_params<'a>(
        request: RequestBuilder<'a>,
        layout: LayoutName,
        result: &AtlanticQueryStep,
    ) -> RequestBuilder<'a> {
        request.form_text("result", &result.to_string()).form_text("layout", layout.to_str())
    }

    /// Parses add job / submit L2 query response
    pub async fn parse_job_response(
        response: Response,
        operation: &str,
    ) -> Result<AtlanticAddJobResponse, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let job_response: AtlanticAddJobResponse =
                response.json().await.map_err(|e| AtlanticError::parse_error(operation, e.to_string()))?;

            debug!(
                operation = operation,
                status = %status,
                job_id = %job_response.atlantic_query_id,
                "Job response parsed successfully"
            );

            Ok(job_response)
        } else {
            let (error_text, status) = extract_http_error_text(response, operation).await;
            debug!(
                operation = operation,
                status = %status,
                error = %error_text,
                "Job request failed"
            );
            Err(AtlanticError::from_http_error_response(operation, status, error_text))
        }
    }

    // ==================== GET BUCKET ====================

    /// Builds a get bucket request (no authentication required)
    pub fn build_get_bucket_request<'a>(builder: RequestBuilder<'a>, bucket_id: &str) -> RequestBuilder<'a> {
        debug!(
            operation = "get_bucket",
            bucket_id = %bucket_id,
            "Building get bucket request"
        );

        builder.method(Method::GET).path("buckets").path(bucket_id)
    }

    /// Parses get bucket response
    pub async fn parse_get_bucket_response(
        response: Response,
        bucket_id: &str,
    ) -> Result<AtlanticGetBucketResponse, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let bucket_response: AtlanticGetBucketResponse =
                response.json().await.map_err(|e| AtlanticError::parse_error("get_bucket", e.to_string()))?;

            debug!(
                operation = "get_bucket",
                bucket_id = %bucket_id,
                status = %status,
                queries_count = bucket_response.queries.len(),
                bucket_status = ?bucket_response.bucket.status,
                "Bucket details retrieved"
            );

            Ok(bucket_response)
        } else {
            let (error_text, status) = extract_http_error_text(response, "get bucket").await;
            debug!(
                operation = "get_bucket",
                bucket_id = %bucket_id,
                status = %status,
                error = %error_text,
                "Failed to get bucket"
            );
            Err(AtlanticError::from_http_error_response("get_bucket", status, error_text))
        }
    }

    // ==================== GET JOB STATUS ====================

    /// Builds a get job status request (no authentication required)
    pub fn build_get_job_status_request<'a>(builder: RequestBuilder<'a>, job_key: &str) -> RequestBuilder<'a> {
        debug!(
            operation = "get_job_status",
            job_key = %job_key,
            "Building get job status request"
        );

        builder.method(Method::GET).path("atlantic-query").path(job_key)
    }

    /// Parses get job status response
    pub async fn parse_get_status_response(
        response: Response,
        job_key: &str,
    ) -> Result<AtlanticGetStatusResponse, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let job_status: AtlanticGetStatusResponse =
                response.json().await.map_err(|e| AtlanticError::parse_error("get_job_status", e.to_string()))?;

            debug!(
                operation = "get_job_status",
                job_key = %job_key,
                status = %status,
                job_status = ?job_status.atlantic_query.status,
                "Job status retrieved"
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
            Err(AtlanticError::from_http_error_response("get_job_status", status, error_text))
        }
    }

    // ==================== SUBMIT L2 QUERY ====================

    /// Builds a submit L2 query request
    ///
    /// This submits a proof for L2 verification.
    pub fn build_submit_l2_query_request<'a>(
        builder: RequestBuilder<'a>,
        auth: &ApiKeyAuth,
        proof: &str,
        job_size: &'static str,
        network: &str,
        program_hash: &str,
    ) -> Result<RequestBuilder<'a>, AtlanticError> {
        debug!(
            operation = "submit_l2_query",
            job_size = job_size,
            network = %network,
            program_hash = %program_hash,
            proof_size_bytes = proof.len(),
            layout = %LayoutName::recursive_with_poseidon.to_str(),
            "Building L2 query request"
        );

        let request = builder
            .method(Method::POST)
            .header(ApiKeyAuth::header_name(), auth.header_value())
            .path("atlantic-query")
            .form_file_bytes("inputFile", proof.as_bytes().to_vec(), "proof.json", Some("application/json"))
            .map_err(|e| AtlanticError::from_io_error("submit_l2_query", e))?
            .form_text("programHash", program_hash)
            .form_text("layout", LayoutName::recursive_with_poseidon.to_str())
            .form_text("declaredJobSize", job_size)
            .form_text("network", network)
            .form_text("result", &AtlanticQueryStep::ProofVerificationOnL2.to_string())
            .form_text("cairoVm", &AtlanticCairoVm::Python.as_str())
            .form_text("cairoVersion", &AtlanticCairoVersion::Cairo0.as_str());

        Ok(request)
    }

    // ==================== GET ARTIFACTS ====================

    /// Parses artifact download response
    ///
    /// Used for downloading proofs, PIE files, and other artifacts from storage.
    pub async fn parse_artifacts_response(response: Response, artifact_path: &str) -> Result<Vec<u8>, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let bytes = response.bytes().await.map_err(|e| AtlanticError::from_reqwest_error("get_artifacts", e))?;

            debug!(
                operation = "get_artifacts",
                url = %artifact_path,
                status = %status,
                artifact_size_bytes = bytes.len(),
                "Artifact downloaded"
            );

            Ok(bytes.to_vec())
        } else {
            let (error_text, status) = extract_http_error_text(response, "get artifacts").await;
            debug!(
                operation = "get_artifacts",
                url = %artifact_path,
                status = %status,
                error = %error_text,
                "Artifact download failed"
            );
            Err(AtlanticError::from_http_error_response("get_artifacts", status, error_text))
        }
    }

    // ==================== GET PROOF BY TASK ID ====================

    /// Parses proof download response
    pub async fn parse_proof_response(
        response: Response,
        task_id: &str,
        proof_path: &str,
    ) -> Result<String, AtlanticError> {
        let status = response.status();

        if status.is_success() {
            let proof_text =
                response.text().await.map_err(|e| AtlanticError::from_reqwest_error("get_proof_by_task_id", e))?;

            debug!(
                operation = "get_proof_by_task_id",
                task_id = %task_id,
                url = %proof_path,
                status = %status,
                proof_size_bytes = proof_text.len(),
                "Proof downloaded"
            );

            Ok(proof_text)
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
            Err(AtlanticError::from_http_error_response("get_proof_by_task_id", status, error_text))
        }
    }

    // ==================== UTILITY ====================

    /// Converts number of steps to job size category
    ///
    /// Based on Atlantic API documentation:
    /// - S: 0-12 million steps
    /// - M: 13-29 million steps
    /// - L: 30+ million steps
    pub fn n_steps_to_job_size(n_steps: Option<usize>) -> &'static str {
        let n_steps = n_steps.unwrap_or(40_000_000) / 1_000_000;

        match n_steps {
            0..=12 => "S",
            13..=29 => "M",
            _ => "L",
        }
    }
}
