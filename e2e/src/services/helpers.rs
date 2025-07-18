use url::Url;
use serde_json::json;
use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum NodeRpcError{
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
        let response = client.post(url)
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

        let json = response.json::<serde_json::Value>().await
            .map_err(|_| NodeRpcError::InvalidResponse)?;

        // Check if there's an error in the JSON-RPC response
        if let Some(error) = json.get("error") {
            // Check for specific "Block not found" error (code 24)
            if let (Some(code), Some(message)) = (error.get("code"), error.get("message")) {
                if code.as_u64() == Some(24) &&
                   message.as_str().map(|s| s.contains("Block not found")).unwrap_or(false) {
                    println!("No blocks mined yet, returning -1");
                    return Ok(-1);
                }
            }

            println!("RPC Error: {:?}", error);
            return Err(NodeRpcError::InvalidResponse);
        }

        // Extract block number directly from result (it's just an integer now)
        let block_number = json.get("result")
            .and_then(|v| v.as_u64())
            .ok_or(NodeRpcError::InvalidResponse)?;

        let block_num_i64 = block_number as i64;

        println!("Madara Block Number: {}", block_num_i64);

        Ok(block_num_i64)
    }

}
