//! Batch JSON-RPC client for efficient Starknet queries.
//!
//! This module provides a batch-capable JSON-RPC client that can send multiple
//! RPC calls in a single HTTP request, significantly reducing network overhead.

use futures::stream::{self, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use starknet_core::types::{BlockId, Felt};
use std::collections::HashMap;
use std::error::Error as StdError;
use std::time::{Duration, Instant};
use thiserror::Error;
use tracing::{debug, error, info, warn};
use url::Url;

/// Default batch size (number of RPC calls per HTTP request)
pub const DEFAULT_BATCH_SIZE: usize = 100;
/// Default number of concurrent batch requests
pub const DEFAULT_MAX_CONCURRENT_BATCHES: usize = 10;
/// Default retry attempts for failed batches
pub const DEFAULT_MAX_RETRIES: u64 = 3;
/// Default delay between retries in seconds
pub const DEFAULT_RETRY_DELAY_SECS: u64 = 2;
/// Default HTTP request timeout in seconds
pub const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;

/// Configuration for batch RPC operations
#[derive(Debug, Clone)]
pub struct BatchRpcConfig {
    /// Maximum number of RPC calls per batch request
    pub batch_size: usize,
    /// Maximum concurrent batch requests
    pub max_concurrent_batches: usize,
    /// Retry attempts for failed batches
    pub max_retries: u64,
    /// Delay between retries in seconds
    pub retry_delay_secs: u64,
    /// HTTP request timeout in seconds
    pub request_timeout_secs: u64,
}

impl Default for BatchRpcConfig {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            max_concurrent_batches: DEFAULT_MAX_CONCURRENT_BATCHES,
            max_retries: DEFAULT_MAX_RETRIES,
            retry_delay_secs: DEFAULT_RETRY_DELAY_SECS,
            request_timeout_secs: DEFAULT_REQUEST_TIMEOUT_SECS,
        }
    }
}

/// Error types for batch RPC operations
#[derive(Error, Debug)]
pub enum BatchRpcError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON serialization failed: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("RPC error for request {id}: code={code}, message={message}")]
    RpcError { id: u64, code: i64, message: String },

    #[error("Missing response for request ID {0}")]
    MissingResponse(u64),

    #[error("All retries exhausted: {0}")]
    RetriesExhausted(String),

    #[error("Invalid response format: {0}")]
    InvalidResponse(String),
}

impl BatchRpcError {
    fn reqwest_error(&self) -> Option<&reqwest::Error> {
        match self {
            Self::HttpError(err) => Some(err),
            _ => None,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::HttpError(_) => "http",
            Self::SerializationError(_) => "serialization",
            Self::RpcError { .. } => "rpc",
            Self::MissingResponse(_) => "missing_response",
            Self::RetriesExhausted(_) => "retries_exhausted",
            Self::InvalidResponse(_) => "invalid_response",
        }
    }
}

/// A single JSON-RPC request
#[derive(Serialize, Debug)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'a str,
    params: serde_json::Value,
    id: u64,
}

/// A single JSON-RPC response
#[derive(Deserialize, Debug)]
struct JsonRpcResponse {
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<JsonRpcErrorData>,
}

#[derive(Deserialize, Debug, Clone)]
struct JsonRpcErrorData {
    code: i64,
    message: String,
}

/// Starknet error code for contract not found
const CONTRACT_NOT_FOUND_CODE: i64 = 20;
const STORAGE_METHOD: &str = "starknet_getStorageAt";
const CLASS_HASH_METHOD: &str = "starknet_getClassHashAt";
const SLOW_BATCH_RPC_LOG_THRESHOLD_MS: u64 = 1_000;
const MAX_ERROR_DETAIL_LEN: usize = 512;

#[derive(Clone, Copy)]
struct BatchRpcLogContext {
    method: &'static str,
    total_query_count: usize,
    chunk_index: usize,
    chunk_count: usize,
    chunk_size: usize,
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

fn sanitize_error_detail(input: &str) -> String {
    let mut out = input.to_string();
    for scheme in ["http://", "https://"] {
        let mut pos = 0;
        while let Some(offset) = out[pos..].find(scheme) {
            let start = pos + offset;
            let end = out[start..]
                .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '}' | ',' | ';'))
                .map(|offset| start + offset)
                .unwrap_or(out.len());
            if let Ok(url) = Url::parse(&out[start..end]) {
                let replacement = redacted_url(&url);
                out.replace_range(start..end, &replacement);
                pos = start + replacement.len();
            } else {
                pos = end;
            }
        }
    }
    if out.len() > MAX_ERROR_DETAIL_LEN {
        let mut end = MAX_ERROR_DETAIL_LEN;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
        out.push_str("...");
    }
    out
}

fn reqwest_source_chain(error: &reqwest::Error) -> String {
    let mut sources = Vec::new();
    let mut current = error.source();

    while let Some(source) = current {
        sources.push(sanitize_error_detail(&source.to_string()));
        current = source.source();
    }

    sources.join(" | ")
}

fn redacted_url(url: &Url) -> String {
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

/// Batch RPC client for efficient Starknet queries
#[derive(Clone)]
pub struct BatchRpcClient {
    client: Client,
    rpc_url: Url,
    config: BatchRpcConfig,
}

impl BatchRpcClient {
    /// Create a new BatchRpcClient
    pub fn new(rpc_url: Url, config: BatchRpcConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.request_timeout_secs))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, rpc_url, config }
    }

    /// Create a new BatchRpcClient with default configuration
    pub fn with_defaults(rpc_url: Url) -> Self {
        Self::new(rpc_url, BatchRpcConfig::default())
    }

    /// Batch get storage values for multiple (contract, key) pairs at a specific block
    ///
    /// Returns a HashMap mapping (contract_address, key) to the storage value.
    /// If a storage slot doesn't exist, ZERO is returned for that slot.
    #[allow(clippy::type_complexity)]
    pub async fn batch_get_storage_at(
        &self,
        queries: Vec<(Felt, Felt)>, // (contract_address, key)
        block_id: BlockId,
    ) -> Result<HashMap<(Felt, Felt), Felt>, BatchRpcError> {
        if queries.is_empty() {
            return Ok(HashMap::new());
        }

        let block_param = serde_json::to_value(block_id).expect("BlockId serialization cannot fail");

        // Pre-chunk queries into owned Vecs. This small allocation (just Vec headers) is necessary
        // because chunks() returns borrowed slices which don't satisfy Send bounds for async.
        let chunks: Vec<Vec<(Felt, Felt)>> = queries.chunks(self.config.batch_size).map(|c| c.to_vec()).collect();

        let total_query_count = queries.len();
        let chunk_count = chunks.len();
        debug!("Executing {} storage queries in {} batches", total_query_count, chunk_count);

        // Execute batches concurrently and merge results incrementally to reduce memory pressure
        let merged = stream::iter(chunks)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                let block_param = block_param.clone();
                let context = BatchRpcLogContext {
                    method: STORAGE_METHOD,
                    total_query_count,
                    chunk_index: chunk_index + 1,
                    chunk_count,
                    chunk_size: chunk.len(),
                };
                async move { self.execute_storage_batch(&chunk, &block_param, context).await }
            })
            .buffer_unordered(self.config.max_concurrent_batches)
            .try_fold(HashMap::new(), |mut acc, batch_result| async move {
                acc.extend(batch_result);
                Ok(acc)
            })
            .await?;

        Ok(merged)
    }

    /// Batch get class hashes for multiple contracts at a specific block
    ///
    /// Returns a HashMap mapping contract_address to Option<class_hash>.
    /// If a contract doesn't exist (ContractNotFound), None is returned.
    pub async fn batch_get_class_hash_at(
        &self,
        contracts: Vec<Felt>,
        block_id: BlockId,
    ) -> Result<HashMap<Felt, Option<Felt>>, BatchRpcError> {
        if contracts.is_empty() {
            return Ok(HashMap::new());
        }

        let block_param = serde_json::to_value(block_id).expect("BlockId serialization cannot fail");

        // Pre-chunk contracts into owned Vecs. This small allocation (just Vec headers) is necessary
        // because chunks() returns borrowed slices which don't satisfy Send bounds for async.
        let chunks: Vec<Vec<Felt>> = contracts.chunks(self.config.batch_size).map(|c| c.to_vec()).collect();

        let total_query_count = contracts.len();
        let chunk_count = chunks.len();
        debug!("Executing {} class hash queries in {} batches", total_query_count, chunk_count);

        // Execute batches concurrently and merge results incrementally to reduce memory pressure
        let merged = stream::iter(chunks)
            .enumerate()
            .map(|(chunk_index, chunk)| {
                let block_param = block_param.clone();
                let context = BatchRpcLogContext {
                    method: CLASS_HASH_METHOD,
                    total_query_count,
                    chunk_index: chunk_index + 1,
                    chunk_count,
                    chunk_size: chunk.len(),
                };
                async move { self.execute_class_hash_batch(&chunk, &block_param, context).await }
            })
            .buffer_unordered(self.config.max_concurrent_batches)
            .try_fold(HashMap::new(), |mut acc, batch_result| async move {
                acc.extend(batch_result);
                Ok(acc)
            })
            .await?;

        Ok(merged)
    }

    /// Execute a batch of storage queries
    async fn execute_storage_batch(
        &self,
        queries: &[(Felt, Felt)],
        block_param: &serde_json::Value,
        context: BatchRpcLogContext,
    ) -> Result<HashMap<(Felt, Felt), Felt>, BatchRpcError> {
        let started_at = Instant::now();

        // Build batch request with positional array params: [contract_address, key, block_id]
        let requests: Vec<JsonRpcRequest<'_>> = queries
            .iter()
            .enumerate()
            .map(|(idx, (contract_addr, key))| JsonRpcRequest {
                jsonrpc: "2.0",
                method: STORAGE_METHOD,
                params: serde_json::json!([format!("{:#x}", contract_addr), format!("{:#x}", key), block_param]),
                id: idx as u64,
            })
            .collect();

        // Send batch with retry
        let responses = self.send_batch_with_retry(&requests, context).await?;
        let rpc_error_count = responses.values().filter(|response| response.is_err()).count();

        // Parse responses
        let mut results = HashMap::new();
        for (idx, (contract_addr, key)) in queries.iter().enumerate() {
            let request_id = idx as u64;
            let response = responses.get(&request_id).ok_or(BatchRpcError::MissingResponse(request_id))?;

            let value = match response {
                Ok(val) => Self::parse_felt_result(val)?,
                Err(err) => {
                    error!(
                        method = context.method,
                        chunk_index = context.chunk_index,
                        chunk_count = context.chunk_count,
                        chunk_size = context.chunk_size,
                        request_id,
                        rpc_error_code = err.code,
                        rpc_error_message_len = err.message.len(),
                        "Batch RPC storage query returned an RPC error"
                    );
                    return Err(BatchRpcError::RpcError {
                        id: request_id,
                        code: err.code,
                        message: err.message.clone(),
                    });
                }
            };
            results.insert((*contract_addr, *key), value);
        }

        let duration_ms = elapsed_ms(started_at);
        if duration_ms >= SLOW_BATCH_RPC_LOG_THRESHOLD_MS {
            info!(
                method = context.method,
                total_query_count = context.total_query_count,
                chunk_index = context.chunk_index,
                chunk_count = context.chunk_count,
                chunk_size = context.chunk_size,
                response_count = responses.len(),
                rpc_error_count,
                duration_ms,
                "Completed slow batch RPC storage chunk"
            );
        }

        Ok(results)
    }

    /// Execute a batch of class hash queries
    async fn execute_class_hash_batch(
        &self,
        contracts: &[Felt],
        block_param: &serde_json::Value,
        context: BatchRpcLogContext,
    ) -> Result<HashMap<Felt, Option<Felt>>, BatchRpcError> {
        let started_at = Instant::now();

        // Build batch request with positional array params: [block_id, contract_address]
        let requests: Vec<JsonRpcRequest<'_>> = contracts
            .iter()
            .enumerate()
            .map(|(idx, contract_addr)| JsonRpcRequest {
                jsonrpc: "2.0",
                method: CLASS_HASH_METHOD,
                params: serde_json::json!([block_param, format!("{:#x}", contract_addr)]),
                id: idx as u64,
            })
            .collect();

        // Send batch with retry
        let responses = self.send_batch_with_retry(&requests, context).await?;
        let rpc_error_count = responses.values().filter(|response| response.is_err()).count();

        // Parse responses
        let mut results = HashMap::new();
        for (idx, contract_addr) in contracts.iter().enumerate() {
            let request_id = idx as u64;
            let response = responses.get(&request_id).ok_or(BatchRpcError::MissingResponse(request_id))?;

            let value = match response {
                Ok(val) => Some(Self::parse_felt_result(val)?),
                Err(err) => {
                    if err.code == CONTRACT_NOT_FOUND_CODE {
                        // Contract doesn't exist, this is expected
                        None
                    } else {
                        // Unexpected error, log but return None to not block processing
                        error!(
                            method = context.method,
                            chunk_index = context.chunk_index,
                            chunk_count = context.chunk_count,
                            chunk_size = context.chunk_size,
                            request_id,
                            rpc_error_code = err.code,
                            rpc_error_message_len = err.message.len(),
                            "Batch RPC class hash query returned an unexpected RPC error"
                        );
                        return Err(BatchRpcError::RpcError {
                            id: request_id,
                            code: err.code,
                            message: err.message.clone(),
                        });
                    }
                }
            };
            results.insert(*contract_addr, value);
        }

        let duration_ms = elapsed_ms(started_at);
        if duration_ms >= SLOW_BATCH_RPC_LOG_THRESHOLD_MS {
            info!(
                method = context.method,
                total_query_count = context.total_query_count,
                chunk_index = context.chunk_index,
                chunk_count = context.chunk_count,
                chunk_size = context.chunk_size,
                response_count = responses.len(),
                rpc_error_count,
                duration_ms,
                "Completed slow batch RPC class hash chunk"
            );
        }

        Ok(results)
    }

    /// Send batch request with retry logic
    async fn send_batch_with_retry(
        &self,
        requests: &[JsonRpcRequest<'_>],
        context: BatchRpcLogContext,
    ) -> Result<HashMap<u64, Result<serde_json::Value, JsonRpcErrorData>>, BatchRpcError> {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < self.config.max_retries {
            attempts += 1;
            let attempt_started_at = Instant::now();

            match self.send_batch(requests, context, attempts).await {
                Ok(responses) => {
                    let rpc_error_count = responses.values().filter(|response| response.is_err()).count();
                    let missing_response_count = Self::missing_response_count(requests, &responses);
                    let duration_ms = elapsed_ms(attempt_started_at);
                    if attempts > 1 || duration_ms >= SLOW_BATCH_RPC_LOG_THRESHOLD_MS || missing_response_count > 0 {
                        info!(
                            method = context.method,
                            total_query_count = context.total_query_count,
                            chunk_index = context.chunk_index,
                            chunk_count = context.chunk_count,
                            chunk_size = context.chunk_size,
                            attempt = attempts,
                            max_retries = self.config.max_retries,
                            duration_ms,
                            response_count = responses.len(),
                            rpc_error_count,
                            missing_response_count,
                            "Batch RPC request attempt succeeded"
                        );
                    }
                    return Ok(responses);
                }
                Err(e) => {
                    let reqwest_error = e.reqwest_error();
                    let error_source_chain = reqwest_error.map(reqwest_source_chain).unwrap_or_default();
                    let reqwest_url = reqwest_error.and_then(|err| err.url()).map(redacted_url).unwrap_or_default();
                    warn!(
                        method = context.method,
                        total_query_count = context.total_query_count,
                        chunk_index = context.chunk_index,
                        chunk_count = context.chunk_count,
                        chunk_size = context.chunk_size,
                        attempt = attempts,
                        max_retries = self.config.max_retries,
                        duration_ms = elapsed_ms(attempt_started_at),
                        error_kind = e.kind(),
                        error_source_chain = %error_source_chain,
                        reqwest_is_error = reqwest_error.is_some(),
                        reqwest_is_timeout = reqwest_error.map(|err| err.is_timeout()).unwrap_or(false),
                        reqwest_is_connect = reqwest_error.map(|err| err.is_connect()).unwrap_or(false),
                        reqwest_is_request = reqwest_error.map(|err| err.is_request()).unwrap_or(false),
                        reqwest_is_body = reqwest_error.map(|err| err.is_body()).unwrap_or(false),
                        reqwest_is_decode = reqwest_error.map(|err| err.is_decode()).unwrap_or(false),
                        reqwest_is_status = reqwest_error.map(|err| err.is_status()).unwrap_or(false),
                        reqwest_status = ?reqwest_error.and_then(|err| err.status()),
                        reqwest_url = %reqwest_url,
                        "Batch RPC request attempt failed"
                    );
                    last_error = Some(e);

                    if attempts < self.config.max_retries {
                        info!(
                            method = context.method,
                            total_query_count = context.total_query_count,
                            chunk_index = context.chunk_index,
                            chunk_count = context.chunk_count,
                            chunk_size = context.chunk_size,
                            next_attempt = attempts + 1,
                            max_retries = self.config.max_retries,
                            retry_delay_secs = self.config.retry_delay_secs,
                            "Retrying batch RPC request after delay"
                        );
                        tokio::time::sleep(Duration::from_secs(self.config.retry_delay_secs)).await;
                    }
                }
            }
        }

        let last_error_ref = last_error.as_ref();
        let last_reqwest_error = last_error_ref.and_then(|err| err.reqwest_error());
        let last_error_kind = last_error_ref.map(BatchRpcError::kind).unwrap_or("unknown");
        let last_error_source_chain = last_reqwest_error.map(reqwest_source_chain).unwrap_or_default();
        let last_reqwest_url = last_reqwest_error.and_then(|err| err.url()).map(redacted_url).unwrap_or_default();

        info!(
            method = context.method,
            total_query_count = context.total_query_count,
            chunk_index = context.chunk_index,
            chunk_count = context.chunk_count,
            chunk_size = context.chunk_size,
            attempts,
            max_retries = self.config.max_retries,
            error_kind = last_error_kind,
            error_source_chain = %last_error_source_chain,
            reqwest_is_error = last_reqwest_error.is_some(),
            reqwest_is_timeout = last_reqwest_error.map(|err| err.is_timeout()).unwrap_or(false),
            reqwest_is_connect = last_reqwest_error.map(|err| err.is_connect()).unwrap_or(false),
            reqwest_is_request = last_reqwest_error.map(|err| err.is_request()).unwrap_or(false),
            reqwest_is_body = last_reqwest_error.map(|err| err.is_body()).unwrap_or(false),
            reqwest_is_decode = last_reqwest_error.map(|err| err.is_decode()).unwrap_or(false),
            reqwest_is_status = last_reqwest_error.map(|err| err.is_status()).unwrap_or(false),
            reqwest_status = ?last_reqwest_error.and_then(|err| err.status()),
            reqwest_url = %last_reqwest_url,
            "Batch RPC retries exhausted"
        );

        Err(BatchRpcError::RetriesExhausted(
            last_error.map(|e| sanitize_error_detail(&e.to_string())).unwrap_or_default(),
        ))
    }

    /// Send a single batch request
    async fn send_batch(
        &self,
        requests: &[JsonRpcRequest<'_>],
        context: BatchRpcLogContext,
        attempt: u64,
    ) -> Result<HashMap<u64, Result<serde_json::Value, JsonRpcErrorData>>, BatchRpcError> {
        let body = serde_json::to_string(requests)?;

        let response = self
            .client
            .post(self.rpc_url.as_str())
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await?
            .error_for_status()?;

        let response_started_at = Instant::now();
        let response_text = response.text().await?;
        let response_duration_ms = elapsed_ms(response_started_at);

        let parse_started_at = Instant::now();
        let responses: Vec<JsonRpcResponse> = serde_json::from_str(&response_text).map_err(|e| {
            error!(
                method = context.method,
                total_query_count = context.total_query_count,
                chunk_index = context.chunk_index,
                chunk_count = context.chunk_count,
                chunk_size = context.chunk_size,
                attempt,
                max_retries = self.config.max_retries,
                response_body_bytes = response_text.len(),
                error = %e,
                "Failed to parse batch RPC response body"
            );
            BatchRpcError::InvalidResponse(e.to_string())
        })?;

        let mut result_map = HashMap::new();
        let mut rpc_error_count = 0;
        let mut invalid_response_count = 0;
        for resp in responses {
            if let Some(err) = resp.error {
                rpc_error_count += 1;
                result_map.insert(resp.id, Err(err));
            } else if let Some(val) = resp.result {
                result_map.insert(resp.id, Ok(val));
            } else {
                invalid_response_count += 1;
                result_map
                    .insert(resp.id, Err(JsonRpcErrorData { code: -1, message: "No result or error".to_string() }));
            }
        }
        let missing_response_count = Self::missing_response_count(requests, &result_map);
        let parse_duration_ms = elapsed_ms(parse_started_at);

        if response_duration_ms >= SLOW_BATCH_RPC_LOG_THRESHOLD_MS
            || parse_duration_ms >= SLOW_BATCH_RPC_LOG_THRESHOLD_MS
            || missing_response_count > 0
            || invalid_response_count > 0
        {
            info!(
                method = context.method,
                total_query_count = context.total_query_count,
                chunk_index = context.chunk_index,
                chunk_count = context.chunk_count,
                chunk_size = context.chunk_size,
                attempt,
                max_retries = self.config.max_retries,
                response_duration_ms,
                parse_duration_ms,
                response_body_bytes = response_text.len(),
                response_count = result_map.len(),
                rpc_error_count,
                missing_response_count,
                invalid_response_count,
                "Parsed notable batch RPC response"
            );
        }

        Ok(result_map)
    }

    fn missing_response_count(
        requests: &[JsonRpcRequest<'_>],
        responses: &HashMap<u64, Result<serde_json::Value, JsonRpcErrorData>>,
    ) -> usize {
        requests.iter().filter(|request| !responses.contains_key(&request.id)).count()
    }

    /// Parse a Felt from JSON-RPC response value
    fn parse_felt_result(value: &serde_json::Value) -> Result<Felt, BatchRpcError> {
        serde_json::from_value(value.clone()).map_err(|e| BatchRpcError::InvalidResponse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_felt_result() {
        let value = serde_json::json!("0x123");
        let felt = BatchRpcClient::parse_felt_result(&value).unwrap();
        assert_eq!(felt, Felt::from_hex("0x123").unwrap());
    }

    #[test]
    fn test_default_config() {
        let config = BatchRpcConfig::default();
        assert_eq!(config.batch_size, 100);
        assert_eq!(config.max_concurrent_batches, 10);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_delay_secs, 2);
        assert_eq!(config.request_timeout_secs, 30);
    }
}
