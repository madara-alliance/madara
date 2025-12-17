//! Chain details fetched from the node at orchestrator startup.
//!
//! This module provides functionality to fetch chain configuration details from:
//! - RPC endpoint (`starknet_chainId`) for chain ID
//! - Feeder gateway (`/feeder_gateway/get_contract_addresses`) for fee token addresses
//!
//! The fetched details are cached in the orchestrator Config and used throughout
//! the application, avoiding redundant network calls during job processing.

use color_eyre::eyre::{eyre, Result};
use serde::Deserialize;
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info};
use url::Url;

/// Retry interval for fetching chain details (default: 5 seconds)
const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// Total timeout for fetching chain details (default: 5 minutes)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

// ============================================================================
// Types
// ============================================================================

/// Response from the feeder gateway `get_contract_addresses` endpoint.
#[derive(Debug, Deserialize)]
pub struct ContractAddressesResponse {
    /// The Starknet core contract address on L1
    #[serde(rename = "Starknet")]
    pub starknet: Option<String>,
    /// The GPS statement verifier contract address
    #[serde(rename = "GpsStatementVerifier")]
    pub gps_statement_verifier: Option<String>,
    /// The ETH fee token address (parent fee token)
    pub eth_l2_token_address: String,
    /// The STRK fee token address (native fee token)
    pub strk_l2_token_address: String,
}

/// Chain details fetched from the node.
///
/// Contains all chain-specific configuration needed by the orchestrator,
/// fetched once at startup and reused throughout the application lifecycle.
#[derive(Debug, Clone)]
pub struct ChainDetails {
    /// The chain ID (e.g., "SN_MAIN", "SN_SEPOLIA")
    pub chain_id: String,
    /// The STRK fee token address (native fee token)
    pub strk_fee_token_address: String,
    /// The ETH fee token address (parent fee token)
    pub eth_fee_token_address: String,
    /// Whether this is an L3 chain (set by caller after fetch)
    pub is_l3: bool,
}

// ============================================================================
// Implementation
// ============================================================================

impl ChainDetails {
    /// Fetch chain details from the node with retry logic.
    ///
    /// This function fetches:
    /// - `chain_id` from the RPC endpoint using `starknet_chainId`
    /// - Fee token addresses from the feeder gateway using `get_contract_addresses`
    ///
    /// Retries every 5 seconds until successful or 5-minute timeout is reached.
    ///
    /// # Arguments
    /// * `rpc_url` - The RPC endpoint URL
    /// * `feeder_gateway_url` - The feeder gateway URL
    ///
    /// # Returns
    /// * `Ok(ChainDetails)` - Successfully fetched chain details
    /// * `Err` - Failed to fetch after timeout
    pub async fn fetch(rpc_url: &Url, feeder_gateway_url: &Url) -> Result<Self> {
        Self::fetch_with_config(rpc_url, feeder_gateway_url, DEFAULT_RETRY_INTERVAL, DEFAULT_TIMEOUT).await
    }

    /// Fetch chain details with custom retry configuration.
    ///
    /// # Arguments
    /// * `rpc_url` - The RPC endpoint URL
    /// * `feeder_gateway_url` - The feeder gateway URL
    /// * `retry_interval` - Duration between retry attempts
    /// * `timeout` - Total timeout duration
    pub async fn fetch_with_config(
        rpc_url: &Url,
        feeder_gateway_url: &Url,
        retry_interval: Duration,
        timeout: Duration,
    ) -> Result<Self> {
        let start_time = Instant::now();
        let mut attempt = 0u32;

        info!("Fetching chain details from node");

        loop {
            attempt += 1;

            match Self::try_fetch(rpc_url, feeder_gateway_url).await {
                Ok(details) => {
                    debug!(
                        chain_id = %details.chain_id,
                        strk_fee_token = %details.strk_fee_token_address,
                        eth_fee_token = %details.eth_fee_token_address,
                        "Chain details fetched"
                    );
                    return Ok(details);
                }
                Err(e) => {
                    let elapsed = start_time.elapsed();

                    if elapsed >= timeout {
                        error!("Failed to fetch chain details after timeout");
                        return Err(eyre!(
                            "Failed to fetch chain details after {} attempts over {} seconds: {}",
                            attempt,
                            elapsed.as_secs(),
                            e
                        ));
                    }

                    info!("Failed to fetch chain details, retrying");
                    debug!(error = %e, "Retry reason");
                    sleep(retry_interval).await;
                }
            }
        }
    }

    /// Attempt to fetch chain details (single attempt, no retry).
    async fn try_fetch(rpc_url: &Url, feeder_gateway_url: &Url) -> Result<Self> {
        let chain_id = Self::fetch_chain_id(rpc_url).await?;
        let addresses = Self::fetch_contract_addresses(feeder_gateway_url).await?;

        Ok(Self {
            chain_id,
            strk_fee_token_address: addresses.strk_l2_token_address,
            eth_fee_token_address: addresses.eth_l2_token_address,
            is_l3: false, // Set by caller after fetch
        })
    }

    /// Fetch chain ID from RPC using `starknet_chainId` method.
    async fn fetch_chain_id(rpc_url: &Url) -> Result<String> {
        let client = reqwest::Client::new();

        let response = client
            .post(rpc_url.as_str())
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "starknet_chainId",
                "params": [],
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| eyre!("Failed to send chain_id request: {}", e))?;

        if !response.status().is_success() {
            return Err(eyre!("Chain ID request failed with status: {}", response.status()));
        }

        let json: serde_json::Value =
            response.json().await.map_err(|e| eyre!("Failed to parse chain_id response: {}", e))?;

        if let Some(error) = json.get("error") {
            return Err(eyre!("RPC error fetching chain_id: {}", error));
        }

        let chain_id_hex =
            json["result"].as_str().ok_or_else(|| eyre!("Invalid chain_id response: missing result field"))?;

        Self::decode_chain_id(chain_id_hex)
    }

    /// Fetch contract addresses from feeder gateway.
    async fn fetch_contract_addresses(feeder_gateway_url: &Url) -> Result<ContractAddressesResponse> {
        let client = reqwest::Client::new();

        let endpoint = feeder_gateway_url
            .join("feeder_gateway/get_contract_addresses")
            .map_err(|e| eyre!("Failed to build contract addresses URL: {}", e))?;

        let response = client
            .get(endpoint.as_str())
            .send()
            .await
            .map_err(|e| eyre!("Failed to send contract addresses request: {}", e))?;

        if !response.status().is_success() {
            return Err(eyre!("Contract addresses request failed with status: {}", response.status()));
        }

        response.json().await.map_err(|e| eyre!("Failed to parse contract addresses response: {}", e))
    }

    /// Decode a hex-encoded chain ID to a string.
    ///
    /// # Example
    /// "0x534e5f4d41494e" -> "SN_MAIN"
    fn decode_chain_id(hex_str: &str) -> Result<String> {
        let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
        let bytes = hex::decode(hex_str).map_err(|e| eyre!("Failed to decode chain_id hex '{}': {}", hex_str, e))?;
        String::from_utf8(bytes).map_err(|e| eyre!("Failed to convert chain_id to UTF-8: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_chain_id() {
        // "SN_MAIN" in hex
        assert_eq!(ChainDetails::decode_chain_id("0x534e5f4d41494e").unwrap(), "SN_MAIN");

        // Without 0x prefix
        assert_eq!(ChainDetails::decode_chain_id("534e5f4d41494e").unwrap(), "SN_MAIN");

        // "SN_SEPOLIA" in hex
        assert_eq!(ChainDetails::decode_chain_id("0x534e5f5345504f4c4941").unwrap(), "SN_SEPOLIA");
    }
}
