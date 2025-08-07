use super::constants::*;
use async_trait::async_trait;
use serde_json::json;
use std::io;
use std::net::TcpListener;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use url::Url;
use thiserror::Error;

/// Error code returned by Starknet RPC when a block is not found
const BLOCK_NOT_FOUND_ERROR_CODE: u64 = 24;

/// Errors that can occur when interacting with a Starknet node RPC
#[derive(Debug, Error)]
pub enum NodeRpcError {
    #[error("Invalid response from RPC endpoint")]
    InvalidResponse,
    #[error("RPC error: {0}")]
    RpcError(String),
    #[error("Block not found")]
    BlockNotFound,
}

/// Transaction finality status from Starknet
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransactionFinalityStatus {
    /// Transaction has been accepted on L2 but not yet on L1
    AcceptedOnL2,
    /// Transaction has been accepted on both L2 and L1
    AcceptedOnL1,
    /// Transaction has been rejected
    Rejected,
}


/// The status of a block in the Starknet blockchain
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BlockStatus {
    /// Block is pending and not yet included in L2
    Pending,
    /// Block has been accepted on L2 but not yet on L1
    AcceptedOnL2,
    /// Block has been accepted on both L2 and L1
    AcceptedOnL1,
    /// Block has been rejected
    Rejected,
}

/// Block identifier for RPC calls
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum BlockId {
    /// Reference to the pending block
    Pending,
    /// Reference to the latest block
    Latest,
    /// Reference to a specific block number
    Number { block_number: u64 },
    /// Reference to a specific block hash
    Hash { block_hash: String },
}

impl From<u64> for BlockId {
    fn from(block_number: u64) -> Self {
        BlockId::Number { block_number }
    }
}

impl From<&str> for BlockId {
    fn from(s: &str) -> Self {
        match s {
            "latest" => BlockId::Latest,
            "pending" => BlockId::Pending,
            hash if hash.starts_with("0x") => BlockId::Hash {
                block_hash: hash.to_string(),
            },
            _ => panic!("Invalid block identifier: {}", s),
        }
    }
}

/// Trait defining RPC methods for interacting with a Starknet node
#[async_trait]
pub trait NodeRpcMethods: Send + Sync {
    /// Returns the RPC endpoint URL for this node
    fn get_endpoint(&self) -> Url;

    /// Fetches the latest block number from the Starknet RPC endpoint.
    ///
    /// # Returns
    ///
    /// * `Ok(block_number)` - The latest block number when blocks exist
    /// * `Err(NodeRpcError::BlockNotFound)` - When no blocks have been mined yet
    /// * `Err(NodeRpcError::InvalidResponse)` - For parsing failures or unexpected responses
    /// * `Err(NodeRpcError::RpcError)` - For network or other RPC errors
    async fn get_latest_block_number(&self) -> Result<Option<u64>, NodeRpcError> {
        match self.make_rpc_request("starknet_blockNumber", json!([])).await {
            Ok(response) => {
                Ok(self.extract_block_number_from_response(&response)?)
            }
            Err(NodeRpcError::BlockNotFound) => Ok(None),
            Err(other_error) => Err(other_error),
        }
    }

    /// Fetches the status of a specific block.
    ///
    /// # Arguments
    ///
    /// * `block_id` - The block identifier (number, hash, or "latest")
    ///
    /// # Returns
    ///
    /// * `Ok(BlockStatus)` - The status of the requested block
    /// * `Err(NodeRpcError::BlockNotFound)` - When the specified block doesn't exist
    /// * `Err(NodeRpcError::InvalidResponse)` - For parsing failures or unexpected responses
    /// * `Err(NodeRpcError::RpcError)` - For network or other RPC errors
    ///
    /// # Examples
    ///
    /// ```rust
    /// # async fn example(node: impl NodeRpcMethods) -> Result<(), NodeRpcError> {
    /// // Get status of latest block
    /// let status = node.get_block_status("latest".into()).await?;
    /// println!("Latest block status: {:?}", status);
    ///
    /// // Get status of specific block number
    /// let status = node.get_block_status(100u64.into()).await?;
    /// println!("Block 100 status: {:?}", status);
    /// # Ok(())
    /// # }
    /// ```
    async fn get_block_status(&self, block_id: BlockId) -> Result<BlockStatus, NodeRpcError> {
        let params = match block_id {
            BlockId::Latest => json!(["latest"]),
            BlockId::Number { block_number } => json!([{"block_number": block_number}]),
            BlockId::Hash { block_hash } => json!([{"block_hash": block_hash}]),
            BlockId::Pending => json!(["pending"]),
        };

        let response = self
            .make_rpc_request("starknet_getBlockWithTxHashes", params)
            .await?;

        self.extract_block_status_from_response(&response)
    }

    /// Fetches the finality status of a specific transaction.
    ///
    /// # Arguments
    ///
    /// * `transaction_hash` - The hash of the transaction to query
    ///
    /// # Returns
    ///
    /// * `Ok(TransactionFinalityStatus)` - The finality status of the transaction
    /// * `Err(NodeRpcError::BlockNotFound)` - When the transaction doesn't exist
    /// * `Err(NodeRpcError::InvalidResponse)` - For parsing failures or unexpected responses
    /// * `Err(NodeRpcError::RpcError)` - For network or other RPC errors
    ///
    /// # Examples
    ///
    /// ```rust
    /// # async fn example(node: impl NodeRpcMethods) -> Result<(), NodeRpcError> {
    /// let tx_hash = "0x778bed983dc662706c623db1b339e2674ebb35da917897738a1a6360186df25";
    /// match node.get_transaction_finality(tx_hash).await {
    ///     Ok(status) => println!("Transaction finality: {:?}", status),
    ///     Err(NodeRpcError::BlockNotFound) => println!("Transaction not found"),
    ///     Err(e) => eprintln!("Error: {}", e),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    async fn get_transaction_finality(&self, transaction_hash: &str) -> Result<TransactionFinalityStatus, NodeRpcError> {
        let response = self
            .make_rpc_request("starknet_getTransactionReceipt", json!([transaction_hash]))
            .await?;

        self.extract_transaction_finality_from_response(&response)
    }

    /// Makes an RPC request to the Starknet node
    ///
    /// # Arguments
    ///
    /// * `method` - The RPC method name
    /// * `params` - The parameters for the RPC call
    ///
    /// # Returns
    ///
    /// * `Ok(Value)` - The JSON response from the RPC endpoint
    /// * `Err(NodeRpcError)` - Various error types depending on failure mode
    async fn make_rpc_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, NodeRpcError> {
        let url = self.get_endpoint();
        let client = reqwest::Client::new();

        let request_body = json!({
            "id": 1,
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let response = client
            .post(url)
            .header("accept", "application/json")
            .header("content-type", "application/json")
            .json(&request_body)
            .send()
            .await
            .map_err(|e| NodeRpcError::RpcError(e.to_string()))?;

        let json = response
            .json::<serde_json::Value>()
            .await
            .map_err(|_| NodeRpcError::InvalidResponse)?;

        // Check for JSON-RPC errors
        if let Some(error) = json.get("error") {
            if let (Some(code), Some(message)) = (error.get("code"), error.get("message")) {
                if code.as_u64() == Some(BLOCK_NOT_FOUND_ERROR_CODE)
                    && message
                        .as_str()
                        .map(|s| s.contains("Block not found"))
                        .unwrap_or(false)
                {
                    return Err(NodeRpcError::BlockNotFound);
                }
            }

            let error_msg = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown RPC error");
            return Err(NodeRpcError::RpcError(error_msg.to_string()));
        }

        // Extract block number directly from result (it's just an integer now)
        let block_number = json.get("result").and_then(|v| v.as_u64()).ok_or(NodeRpcError::InvalidResponse)?;
        Ok(block_number)
    }

    /// Extracts block number from RPC response
    fn extract_block_number_from_response(
        &self,
        response: &serde_json::Value,
    ) -> Result<Option<u64>, NodeRpcError> {
        response
            .get("result")
            .and_then(|v| Some(v.as_u64()))
            .ok_or(NodeRpcError::InvalidResponse)
    }

    /// Extracts block status from RPC response
    fn extract_block_status_from_response(
        &self,
        response: &serde_json::Value,
    ) -> Result<BlockStatus, NodeRpcError> {
        let status_str = response
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(|status| status.as_str())
            .ok_or(NodeRpcError::InvalidResponse)?;

        // Parse the status string into our enum
        serde_json::from_str(&format!("\"{}\"", status_str))
            .map_err(|_| NodeRpcError::InvalidResponse)
    }

    /// Extracts transaction finality status from RPC response
    fn extract_transaction_finality_from_response(
        &self,
        response: &serde_json::Value,
    ) -> Result<TransactionFinalityStatus, NodeRpcError> {
        let finality_str = response
            .get("result")
            .and_then(|result| result.get("finality_status"))
            .and_then(|status| status.as_str())
            .ok_or(NodeRpcError::InvalidResponse)?;

        // Parse the finality status string into our enum
        serde_json::from_str(&format!("\"{}\"", finality_str))
            .map_err(|_| NodeRpcError::InvalidResponse)
    }
}


lazy_static::lazy_static! {
    static ref ALLOCATED_PORTS: Mutex<HashSet<u16>> = Mutex::new(HashSet::new());
}

/// Get a free port and reserve it to prevent double allocation
pub fn get_free_port() -> Result<u16, io::Error> {
    let mut allocated = ALLOCATED_PORTS.lock().unwrap();

    loop {
        let listener = TcpListener::bind(format!("{}:0", DEFAULT_SERVICE_HOST))?;
        let port = listener.local_addr()?.port();

        // Check if we've already allocated this port
        if !allocated.contains(&port) {
            allocated.insert(port);
            return Ok(port);
        }
        // If port was already allocated, try again
    }
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
