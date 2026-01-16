//! Batch JSON-RPC client for efficient Starknet queries.
//!
//! This module provides a batch-capable JSON-RPC client that can send multiple
//! RPC calls in a single HTTP request, significantly reducing network overhead.

use futures::stream::{self, StreamExt, TryStreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use starknet_core::types::{BlockId, Felt};
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, warn};
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

        debug!("Executing {} storage queries in {} batches", queries.len(), chunks.len());

        // Execute batches concurrently and merge results incrementally to reduce memory pressure
        let merged = stream::iter(chunks)
            .map(|chunk| {
                let block_param = block_param.clone();
                async move { self.execute_storage_batch(&chunk, &block_param).await }
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

        debug!("Executing {} class hash queries in {} batches", contracts.len(), chunks.len());

        // Execute batches concurrently and merge results incrementally to reduce memory pressure
        let merged = stream::iter(chunks)
            .map(|chunk| {
                let block_param = block_param.clone();
                async move { self.execute_class_hash_batch(&chunk, &block_param).await }
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
    ) -> Result<HashMap<(Felt, Felt), Felt>, BatchRpcError> {
        // Build batch request with positional array params: [contract_address, key, block_id]
        let requests: Vec<JsonRpcRequest<'_>> = queries
            .iter()
            .enumerate()
            .map(|(idx, (contract_addr, key))| JsonRpcRequest {
                jsonrpc: "2.0",
                method: "starknet_getStorageAt",
                params: serde_json::json!([format!("{:#x}", contract_addr), format!("{:#x}", key), block_param]),
                id: idx as u64,
            })
            .collect();

        // Send batch with retry
        let responses = self.send_batch_with_retry(&requests).await?;

        // Parse responses
        let mut results = HashMap::new();
        for (idx, (contract_addr, key)) in queries.iter().enumerate() {
            let response = responses.get(&(idx as u64)).ok_or(BatchRpcError::MissingResponse(idx as u64))?;

            let value = match response {
                Ok(val) => Self::parse_felt_result(val)?,
                Err(err) => {
                    error!(
                        "Failed to get pre-range storage value for contract: {}, key: {} at block {}: {:?}",
                        contract_addr, key, block_param, err
                    );
                    return Err(BatchRpcError::RpcError {
                        id: idx as u64,
                        code: err.code,
                        message: err.message.clone(),
                    });
                }
            };
            results.insert((*contract_addr, *key), value);
        }

        Ok(results)
    }

    /// Execute a batch of class hash queries
    async fn execute_class_hash_batch(
        &self,
        contracts: &[Felt],
        block_param: &serde_json::Value,
    ) -> Result<HashMap<Felt, Option<Felt>>, BatchRpcError> {
        // Build batch request with positional array params: [block_id, contract_address]
        let requests: Vec<JsonRpcRequest<'_>> = contracts
            .iter()
            .enumerate()
            .map(|(idx, contract_addr)| JsonRpcRequest {
                jsonrpc: "2.0",
                method: "starknet_getClassHashAt",
                params: serde_json::json!([block_param, format!("{:#x}", contract_addr)]),
                id: idx as u64,
            })
            .collect();

        // Send batch with retry
        let responses = self.send_batch_with_retry(&requests).await?;

        // Parse responses
        let mut results = HashMap::new();
        for (idx, contract_addr) in contracts.iter().enumerate() {
            let response = responses.get(&(idx as u64)).ok_or(BatchRpcError::MissingResponse(idx as u64))?;

            let value = match response {
                Ok(val) => Some(Self::parse_felt_result(val)?),
                Err(err) => {
                    if err.code == CONTRACT_NOT_FOUND_CODE {
                        // Contract doesn't exist, this is expected
                        None
                    } else {
                        // Unexpected error, log but return None to not block processing
                        let err_message = format!(
                            "Failed to get class hash for contract: {} at block {}: {:?}",
                            contract_addr, block_param, err
                        );
                        error!("{}", &err_message);
                        return Err(BatchRpcError::RpcError {
                            id: idx as u64,
                            code: err.code,
                            message: err.message.clone(),
                        });
                    }
                }
            };
            results.insert(*contract_addr, value);
        }

        Ok(results)
    }

    /// Send batch request with retry logic
    async fn send_batch_with_retry(
        &self,
        requests: &[JsonRpcRequest<'_>],
    ) -> Result<HashMap<u64, Result<serde_json::Value, JsonRpcErrorData>>, BatchRpcError> {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < self.config.max_retries {
            match self.send_batch(requests).await {
                Ok(responses) => return Ok(responses),
                Err(e) => {
                    attempts += 1;
                    warn!("Batch RPC request failed (attempt {}/{}): {}", attempts, self.config.max_retries, e);
                    last_error = Some(e);

                    if attempts < self.config.max_retries {
                        tokio::time::sleep(Duration::from_secs(self.config.retry_delay_secs)).await;
                    }
                }
            }
        }

        Err(BatchRpcError::RetriesExhausted(last_error.map(|e| e.to_string()).unwrap_or_default()))
    }

    /// Send a single batch request
    async fn send_batch(
        &self,
        requests: &[JsonRpcRequest<'_>],
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

        let response_text = response.text().await?;
        let responses: Vec<JsonRpcResponse> = serde_json::from_str(&response_text).map_err(|e| {
            error!("Failed to parse batch response: {}", response_text);
            BatchRpcError::InvalidResponse(e.to_string())
        })?;

        let mut result_map = HashMap::new();
        for resp in responses {
            if let Some(err) = resp.error {
                result_map.insert(resp.id, Err(err));
            } else if let Some(val) = resp.result {
                result_map.insert(resp.id, Ok(val));
            } else {
                result_map
                    .insert(resp.id, Err(JsonRpcErrorData { code: -1, message: "No result or error".to_string() }));
            }
        }

        Ok(result_map)
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
