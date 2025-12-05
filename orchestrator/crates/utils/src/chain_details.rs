//! Chain details fetched from the node at orchestrator startup.
//!
//! This module provides functionality to fetch chain configuration details from:
//! - RPC endpoint (`starknet_chainId`) for chain ID
//! - Feeder gateway (`/feeder_gateway/get_contract_addresses`) for fee token addresses
//!
//! The fetched details are cached in the orchestrator Config and used throughout
//! the application, avoiding redundant network calls during job processing.

use crate::layer::Layer;
use crate::metrics::lib::{register_counter_metric_instrument, register_gauge_metric_instrument, Metrics};
use crate::register_metric;
use color_eyre::eyre::{eyre, Result};
use once_cell::sync::Lazy;
use opentelemetry::global;
use opentelemetry::metrics::{Counter, Gauge};
use opentelemetry::KeyValue;
use serde::Deserialize;
use std::time::Duration;
use tokio::time::{sleep, Instant};
use tracing::{debug, error, info, warn};
use url::Url;

/// Retry interval for fetching chain details (default: 5 seconds)
const DEFAULT_RETRY_INTERVAL: Duration = Duration::from_secs(5);
/// Total timeout for fetching chain details (default: 5 minutes)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

// ============================================================================
// Metrics
// ============================================================================

register_metric!(CHAIN_DETAILS_METRICS, ChainDetailsMetrics);

/// Metrics for chain details fetch operations.
pub struct ChainDetailsMetrics {
    /// Counter for failed fetch attempts (incremented on each retry)
    pub fetch_failures: Counter<f64>,
    /// Counter for successful fetches
    pub fetch_successes: Counter<f64>,
    /// Gauge for the number of retry attempts in the current/last fetch operation
    pub retry_attempts: Gauge<f64>,
    /// Gauge for the total time taken to fetch chain details (in seconds)
    pub fetch_duration_seconds: Gauge<f64>,
}

impl Metrics for ChainDetailsMetrics {
    fn register() -> Self {
        let meter = global::meter("crates.orchestrator.chain_details");

        let fetch_failures = register_counter_metric_instrument(
            &meter,
            "chain_details_fetch_failures".to_string(),
            "Number of failed attempts to fetch chain details from node".to_string(),
            "failures".to_string(),
        );

        let fetch_successes = register_counter_metric_instrument(
            &meter,
            "chain_details_fetch_successes".to_string(),
            "Number of successful chain details fetches".to_string(),
            "successes".to_string(),
        );

        let retry_attempts = register_gauge_metric_instrument(
            &meter,
            "chain_details_retry_attempts".to_string(),
            "Number of retry attempts for the current/last fetch operation".to_string(),
            "attempts".to_string(),
        );

        let fetch_duration_seconds = register_gauge_metric_instrument(
            &meter,
            "chain_details_fetch_duration_seconds".to_string(),
            "Time taken to successfully fetch chain details".to_string(),
            "seconds".to_string(),
        );

        Self { fetch_failures, fetch_successes, retry_attempts, fetch_duration_seconds }
    }
}

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
    /// Whether this is an L3 chain
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
    /// * `layer` - The layer configuration (L2 or L3)
    ///
    /// # Returns
    /// * `Ok(ChainDetails)` - Successfully fetched chain details
    /// * `Err` - Failed to fetch after timeout
    pub async fn fetch(rpc_url: &Url, feeder_gateway_url: &Url, layer: &Layer) -> Result<Self> {
        Self::fetch_with_config(rpc_url, feeder_gateway_url, layer, DEFAULT_RETRY_INTERVAL, DEFAULT_TIMEOUT).await
    }

    /// Fetch chain details with custom retry configuration.
    ///
    /// # Arguments
    /// * `rpc_url` - The RPC endpoint URL
    /// * `feeder_gateway_url` - The feeder gateway URL
    /// * `layer` - The layer configuration (L2 or L3)
    /// * `retry_interval` - Duration between retry attempts
    /// * `timeout` - Total timeout duration
    pub async fn fetch_with_config(
        rpc_url: &Url,
        feeder_gateway_url: &Url,
        layer: &Layer,
        retry_interval: Duration,
        timeout: Duration,
    ) -> Result<Self> {
        let start_time = Instant::now();
        let mut attempt = 0u32;

        info!(
            rpc_url = %rpc_url,
            feeder_gateway_url = %feeder_gateway_url,
            layer = ?layer,
            retry_interval_secs = retry_interval.as_secs(),
            timeout_secs = timeout.as_secs(),
            "Starting chain details fetch with retry"
        );

        loop {
            attempt += 1;

            // Record current attempt count
            CHAIN_DETAILS_METRICS.retry_attempts.record(attempt as f64, &[]);

            debug!(
                attempt = attempt,
                elapsed_secs = start_time.elapsed().as_secs(),
                "Attempting to fetch chain details"
            );

            match Self::try_fetch(rpc_url, feeder_gateway_url, layer).await {
                Ok(details) => {
                    let elapsed = start_time.elapsed();

                    // Record success metrics
                    CHAIN_DETAILS_METRICS.fetch_successes.add(1.0, &[]);
                    CHAIN_DETAILS_METRICS.fetch_duration_seconds.record(elapsed.as_secs_f64(), &[]);

                    if attempt > 1 {
                        info!(
                            attempts = attempt,
                            elapsed_secs = elapsed.as_secs(),
                            chain_id = %details.chain_id,
                            "Successfully fetched chain details after retries"
                        );
                    } else {
                        info!(
                            chain_id = %details.chain_id,
                            elapsed_ms = elapsed.as_millis(),
                            "Successfully fetched chain details on first attempt"
                        );
                    }

                    debug!(
                        chain_id = %details.chain_id,
                        strk_fee_token = %details.strk_fee_token_address,
                        eth_fee_token = %details.eth_fee_token_address,
                        is_l3 = details.is_l3,
                        "Chain details retrieved"
                    );

                    return Ok(details);
                }
                Err(e) => {
                    let elapsed = start_time.elapsed();

                    // Record failure metric with error context
                    CHAIN_DETAILS_METRICS.fetch_failures.add(
                        1.0,
                        &[
                            KeyValue::new("attempt", attempt as i64),
                            KeyValue::new("error_type", Self::categorize_error(&e)),
                        ],
                    );

                    if elapsed >= timeout {
                        error!(
                            attempts = attempt,
                            elapsed_secs = elapsed.as_secs(),
                            timeout_secs = timeout.as_secs(),
                            error = %e,
                            rpc_url = %rpc_url,
                            feeder_gateway_url = %feeder_gateway_url,
                            "Failed to fetch chain details after timeout - orchestrator cannot start"
                        );
                        return Err(eyre!(
                            "Failed to fetch chain details after {} attempts over {} seconds: {}",
                            attempt,
                            elapsed.as_secs(),
                            e
                        ));
                    }

                    warn!(
                        attempt = attempt,
                        elapsed_secs = elapsed.as_secs(),
                        remaining_secs = (timeout - elapsed).as_secs(),
                        error = %e,
                        retry_in_secs = retry_interval.as_secs(),
                        "Failed to fetch chain details, will retry"
                    );

                    sleep(retry_interval).await;
                }
            }
        }
    }

    /// Attempt to fetch chain details (single attempt, no retry).
    async fn try_fetch(rpc_url: &Url, feeder_gateway_url: &Url, layer: &Layer) -> Result<Self> {
        debug!(rpc_url = %rpc_url, "Fetching chain_id from RPC");

        // Fetch chain_id from RPC
        let chain_id = Self::fetch_chain_id(rpc_url).await?;
        debug!(chain_id = %chain_id, "Successfully fetched chain_id");

        debug!(feeder_gateway_url = %feeder_gateway_url, "Fetching contract addresses from feeder gateway");

        // Fetch contract addresses from feeder gateway
        let addresses = Self::fetch_contract_addresses(feeder_gateway_url).await?;
        debug!(
            strk_token = %addresses.strk_l2_token_address,
            eth_token = %addresses.eth_l2_token_address,
            "Successfully fetched contract addresses"
        );

        Ok(Self {
            chain_id,
            strk_fee_token_address: addresses.strk_l2_token_address,
            eth_fee_token_address: addresses.eth_l2_token_address,
            is_l3: layer.is_l3(),
        })
    }

    /// Fetch chain ID from RPC using `starknet_chainId` method.
    async fn fetch_chain_id(rpc_url: &Url) -> Result<String> {
        let client = reqwest::Client::new();

        debug!(url = %rpc_url, method = "starknet_chainId", "Sending RPC request");

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
            .map_err(|e| eyre!("Failed to send chain_id request to {}: {}", rpc_url, e))?;

        let status = response.status();
        debug!(status = %status, "Received RPC response");

        if !status.is_success() {
            return Err(eyre!("Chain ID request to {} failed with status: {}", rpc_url, status));
        }

        let json: serde_json::Value =
            response.json().await.map_err(|e| eyre!("Failed to parse chain_id response: {}", e))?;

        // Check for JSON-RPC error
        if let Some(error) = json.get("error") {
            return Err(eyre!("RPC error fetching chain_id: {}", error));
        }

        let chain_id_hex =
            json["result"].as_str().ok_or_else(|| eyre!("Invalid chain_id response: missing result field"))?;

        debug!(chain_id_hex = %chain_id_hex, "Received chain_id in hex format");

        // Decode hex chain ID to string (e.g., "0x534e5f4d41494e" -> "SN_MAIN")
        let chain_id_decoded = Self::decode_chain_id(chain_id_hex)?;

        Ok(chain_id_decoded)
    }

    /// Fetch contract addresses from feeder gateway.
    async fn fetch_contract_addresses(feeder_gateway_url: &Url) -> Result<ContractAddressesResponse> {
        let client = reqwest::Client::new();

        // Build the endpoint URL
        let endpoint = feeder_gateway_url
            .join("feeder_gateway/get_contract_addresses")
            .map_err(|e| eyre!("Failed to build contract addresses URL: {}", e))?;

        debug!(url = %endpoint, "Sending feeder gateway request");

        let response = client
            .get(endpoint.as_str())
            .send()
            .await
            .map_err(|e| eyre!("Failed to send contract addresses request to {}: {}", endpoint, e))?;

        let status = response.status();
        debug!(status = %status, "Received feeder gateway response");

        if !status.is_success() {
            return Err(eyre!("Contract addresses request to {} failed with status: {}", endpoint, status));
        }

        let addresses: ContractAddressesResponse =
            response.json().await.map_err(|e| eyre!("Failed to parse contract addresses response: {}", e))?;

        Ok(addresses)
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

    /// Categorize an error for metrics purposes.
    fn categorize_error(error: &color_eyre::eyre::Error) -> String {
        let error_str = error.to_string().to_lowercase();
        if error_str.contains("timeout") || error_str.contains("timed out") {
            "timeout".to_string()
        } else if error_str.contains("connection") || error_str.contains("connect") {
            "connection".to_string()
        } else if error_str.contains("dns") || error_str.contains("resolve") {
            "dns".to_string()
        } else if error_str.contains("status") {
            "http_error".to_string()
        } else if error_str.contains("parse") || error_str.contains("json") {
            "parse_error".to_string()
        } else {
            "unknown".to_string()
        }
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

    #[test]
    fn test_categorize_error() {
        assert_eq!(ChainDetails::categorize_error(&eyre!("Connection refused")), "connection");
        assert_eq!(ChainDetails::categorize_error(&eyre!("Request timed out")), "timeout");
        assert_eq!(ChainDetails::categorize_error(&eyre!("DNS resolution failed")), "dns");
        assert_eq!(ChainDetails::categorize_error(&eyre!("HTTP status 500")), "http_error");
        assert_eq!(ChainDetails::categorize_error(&eyre!("Failed to parse JSON")), "parse_error");
        assert_eq!(ChainDetails::categorize_error(&eyre!("Something else")), "unknown");
    }
}
