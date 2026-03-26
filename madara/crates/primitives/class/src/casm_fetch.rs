//! Dynamic CASM fetching for Sierra classes that fail to compile locally.
//!
//! When a Sierra class fails to compile with the local compiler (e.g., due to
//! version mismatches), this module provides functionality to fetch the pre-compiled
//! CASM from a remote Starknet RPC endpoint.
//!
//! This is a fallback mechanism that allows sync to continue even when local
//! compilation fails for certain classes.

use casm_classes_v2::casm_contract_class::CasmContractClass;
use serde::{Deserialize, Serialize};
use starknet_types_core::felt::Felt;
use std::time::Duration;

/// Timeout for RPC requests
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Error type for CASM fetching operations
#[derive(Debug, thiserror::Error)]
pub enum CasmFetchError {
    #[error("HTTP request failed: {0}")]
    HttpError(String),

    #[error("Failed to parse JSON response: {0}")]
    JsonParseError(String),

    #[error("RPC error: code={code}, message={message}")]
    RpcError { code: i64, message: String },

    #[error("All RPC endpoints failed")]
    AllEndpointsFailed,

    #[error("No compiled CASM found for class")]
    NotFound,

    #[error("No RPC endpoints configured")]
    NoEndpoints,
}

#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    method: &'static str,
    params: JsonRpcParams<'a>,
    id: u32,
}

#[derive(Serialize)]
struct JsonRpcParams<'a> {
    class_hash: &'a str,
}

#[derive(Deserialize)]
struct JsonRpcResponse {
    result: Option<CasmContractClass>,
    error: Option<JsonRpcError>,
}

#[derive(Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

/// Fetches compiled CASM for a Sierra class from remote RPC endpoints.
///
/// Tries each endpoint in order until one succeeds. Uses the `starknet_getCompiledCasm`
/// RPC method.
///
/// # Arguments
/// * `class_hash` - The class hash to fetch compiled CASM for
/// * `endpoints` - List of RPC endpoint URLs to try
///
/// # Returns
/// * `Ok(CasmContractClass)` - The compiled CASM if found
/// * `Err(CasmFetchError)` - If all endpoints fail or class not found
pub fn fetch_compiled_casm(class_hash: &Felt, endpoints: &[&str]) -> Result<CasmContractClass, CasmFetchError> {
    if endpoints.is_empty() {
        return Err(CasmFetchError::NoEndpoints);
    }

    let class_hash_hex = format!("{:#x}", class_hash);

    let request = JsonRpcRequest {
        jsonrpc: "2.0",
        method: "starknet_getCompiledCasm",
        params: JsonRpcParams { class_hash: &class_hash_hex },
        id: 1,
    };

    let request_body = serde_json::to_string(&request).map_err(|e| CasmFetchError::JsonParseError(e.to_string()))?;

    let mut last_error = CasmFetchError::AllEndpointsFailed;

    for endpoint in endpoints {
        tracing::debug!("Trying to fetch compiled CASM from {}", endpoint);

        match fetch_from_endpoint(endpoint, &request_body) {
            Ok(casm) => {
                tracing::info!("Successfully fetched compiled CASM for class {:#x} from {}", class_hash, endpoint);
                return Ok(casm);
            }
            Err(e) => {
                tracing::debug!("Failed to fetch from {}: {}", endpoint, e);
                last_error = e;
            }
        }
    }

    Err(last_error)
}

/// Performs synchronous HTTP request to fetch CASM from a single endpoint.
fn fetch_from_endpoint(endpoint: &str, request_body: &str) -> Result<CasmContractClass, CasmFetchError> {
    let response = ureq::post(endpoint)
        .set("Content-Type", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .send_string(request_body)
        .map_err(|e| CasmFetchError::HttpError(e.to_string()))?;

    let response_text = response.into_string().map_err(|e| CasmFetchError::HttpError(e.to_string()))?;

    let rpc_response: JsonRpcResponse = serde_json::from_str(&response_text).map_err(|e| {
        CasmFetchError::JsonParseError(format!("{}: {}", e, &response_text[..200.min(response_text.len())]))
    })?;

    if let Some(error) = rpc_response.error {
        return Err(CasmFetchError::RpcError { code: error.code, message: error.message });
    }

    rpc_response.result.ok_or(CasmFetchError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "Requires network access"]
    fn test_fetch_compiled_casm() {
        let class_hash = Felt::from_hex_unchecked("0x78d552edf8d22e566b050c9158d7f770b55021c36a9f5a98170ff8fcabc9e10");
        let endpoints = ["https://free-rpc.nethermind.io/mainnet-juno/"];

        let result = fetch_compiled_casm(&class_hash, &endpoints);
        assert!(result.is_ok(), "Should fetch compiled CASM: {:?}", result);

        let casm = result.unwrap();
        assert!(!casm.bytecode.is_empty(), "CASM should have bytecode");
    }
}
