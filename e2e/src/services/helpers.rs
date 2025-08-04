use crate::services::constants::DEFAULT_SERVICE_HOST;
use async_trait::async_trait;
use serde_json::json;
use std::io;
use std::net::TcpListener;
use std::path::PathBuf;
use url::Url;

use super::constants::*;


#[derive(Debug, thiserror::Error)]
pub enum NodeRpcError {
    #[error("Invalid response")]
    InvalidResponse,
}

#[async_trait]
pub trait NodeRpcMethods: Send + Sync {
    fn get_endpoint(&self) -> Url;

    /// Fetches the latest block number from the Starknet RPC endpoint.
    ///
    /// Returns:
    /// - `Ok(-1)` when no blocks have been mined yet (RPC returns "Block not found" error with code 24)
    /// - `Ok(block_number)` when blocks exist
    /// - `Err(NodeRpcError::InvalidResponse)` for other RPC errors or parsing failures
    ///
    /// Note: The -1 return value indicates that the blockchain is in an initial state
    /// with no blocks mined, which is common when first starting a node or testnet.
    async fn get_latest_block_number(&self) -> Result<i64, NodeRpcError> {
        let url = self.get_endpoint();
        let client = reqwest::Client::new();
        let response = client
            .post(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .json(&json!({
                "id": 1,
                "jsonrpc": "2.0",
                "method": "starknet_blockNumber",
                "params": []
            }))
            .send()
            .await
            .map_err(|_| NodeRpcError::InvalidResponse)?;

        let json = response.json::<serde_json::Value>().await.map_err(|_| NodeRpcError::InvalidResponse)?;

        // Check if there's an error in the JSON-RPC response
        if let Some(error) = json.get("error") {
            // Check for specific "Block not found" error (code 24)
            if let (Some(code), Some(message)) = (error.get("code"), error.get("message")) {
                if code.as_u64() == Some(BLOCK_NOT_FOUND_ERROR_CODE)
                    && message.as_str().map(|s| s.contains("Block not found")).unwrap_or(false)
                {
                    println!("No blocks mined yet, returning -1");
                    return Ok(-1);
                }
            }

            println!("RPC Error: {:?}", error);
            return Err(NodeRpcError::InvalidResponse);
        }

        // Extract block number directly from result (it's just an integer now)
        let block_number = json.get("result").and_then(|v| v.as_u64()).ok_or(NodeRpcError::InvalidResponse)?;

        let block_num_i64 = block_number as i64;

        println!("Madara Block Number: {}", block_num_i64);

        Ok(block_num_i64)
    }
}

/// Get a free port
pub fn get_free_port() -> Result<u16, io::Error> {
    let listener = TcpListener::bind(format!("{}:0", DEFAULT_SERVICE_HOST))?;
    let addr = listener.local_addr()?;
    Ok(addr.port())
}

/// Get the binary path
pub fn get_binary_path(binary_name: &str) -> PathBuf {
    let mut path = REPO_ROOT.clone();
    path.push(BINARY_DIR);
    path.push(binary_name);
    path
}

pub fn get_file_path(file_path: &str) -> PathBuf {
    let mut path = REPO_ROOT.clone();
    path.push(file_path);
    path
}

pub fn get_database_path(database_path: &str, database_name: &str) -> PathBuf {
    let mut path = REPO_ROOT.clone();
    path.push(database_path);
    path.push(database_name);
    path
}

pub fn get_container_name(name: &str) -> String {
    let uuid_num = uuid::Uuid::new_v4().as_u128();
    let random_suffix = (uuid_num % 9000 + 1000) as u16;
    format!("{}-{}", name, random_suffix)
}

pub fn docker_url_conversion(url: &Url) -> Url {
    // Convert a localhost / 0.0.0.0 / 127.0.0.1
    // Url to host.docker.internal
    if url.host_str() == Some("localhost") || url.host_str() == Some("0.0.0.0") || url.host_str() == Some("127.0.0.1") {
        let mut new_url = url.clone();
        let _ = new_url.set_host(Some("host.docker.internal"));
        new_url
    } else {
        url.clone()
    }
}
