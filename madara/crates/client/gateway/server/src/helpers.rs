use std::collections::HashMap;

use hyper::{body::Incoming, header, Request, Response, StatusCode};
use mp_block::{BlockId, BlockTag};
use mp_gateway::error::{StarknetError, StarknetErrorCode};
use serde::Serialize;
use starknet_types_core::felt::Felt;

pub(crate) fn service_unavailable_response(service_name: &str) -> Response<String> {
    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .body(format!("{} Service disabled", service_name))
        .expect("Failed to build SERVICE_UNAVAILABLE response with a valid status and body")
}

pub(crate) fn not_found_response() -> Response<String> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body("Not Found".to_string())
        .expect("Failed to build NOT_FOUND response with a valid status and body")
}

pub(crate) fn internal_error_response() -> Response<String> {
    Response::builder()
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body("Internal Server Error".to_string())
        .expect("Failed to build INTERNAL_SERVER_ERROR response with a valid status and body")
}

/// Creates a JSON response with the given status code and a body that can be serialized to JSON.
///
/// If the serialization fails, this function returns a 500 Internal Server Error response.
pub(crate) fn create_json_response<T>(status: StatusCode, body: &T) -> Response<String>
where
    T: Serialize,
{
    // Serialize the body to JSON
    let body = match serde_json::to_string(body) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("Failed to serialize response body: {}", e);
            return internal_error_response();
        }
    };

    // Build the response with the specified status code and serialized body
    match Response::builder().status(status).header(header::CONTENT_TYPE, "application/json").body(body) {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to build response: {}", e);
            internal_error_response()
        }
    }
}

/// Creates a JSON response with the given status code and a body that can be serialized to JSON.
///
/// If the serialization fails, this function returns a 500 Internal Server Error response.
pub(crate) fn create_string_response(status: StatusCode, body: String) -> Response<String> {
    // Build the response with the specified status code and serialized body
    match Response::builder().status(status).body(body) {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to build response: {}", e);
            internal_error_response()
        }
    }
}

/// Creates a JSON response with the given status code and a body that is already serialized to a string.
pub(crate) fn create_response_with_json_body(status: StatusCode, body: String) -> Response<String> {
    // Build the response with the specified status code and serialized body
    match Response::builder().status(status).header(header::CONTENT_TYPE, "application/json").body(body) {
        Ok(response) => response,
        Err(e) => {
            tracing::error!("Failed to build response: {}", e);
            internal_error_response()
        }
    }
}

pub(crate) fn get_params_from_request(req: &Request<Incoming>) -> HashMap<String, String> {
    let query = req.uri().query().unwrap_or("");
    let params = query.split('&');
    let mut query_params = HashMap::new();
    for param in params {
        let parts: Vec<&str> = param.split('=').collect();
        if parts.len() == 2 {
            query_params.insert(parts[0].to_string(), parts[1].to_string());
        }
    }
    query_params
}

pub(crate) fn block_id_from_params(params: &HashMap<String, String>) -> Result<BlockId, StarknetError> {
    if let Some(block_number) = params.get("blockNumber") {
        match block_number.as_str() {
            "latest" => Ok(BlockId::Tag(BlockTag::Latest)),
            "pending" => Ok(BlockId::Tag(BlockTag::Pending)),
            _ => {
                let block_number = block_number.parse().map_err(|e: std::num::ParseIntError| {
                    StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string())
                })?;
                Ok(BlockId::Number(block_number))
            }
        }
    } else if let Some(block_hash) = params.get("blockHash") {
        let block_hash = Felt::from_hex(block_hash)
            .map_err(|e| StarknetError::new(StarknetErrorCode::MalformedRequest, e.to_string()))?;
        Ok(BlockId::Hash(block_hash))
    } else {
        // latest is implicit
        Ok(BlockId::Tag(BlockTag::Latest))
    }
}

pub(crate) fn include_block_params(params: &HashMap<String, String>) -> bool {
    params.get("includeBlock").is_some_and(|v| v == "true")
}
