use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::networks::{ChainType, NetworkRegistry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementConfig {
    /// Network name (e.g., "ethereum-sepolia", "starknet-mainnet", "ethereum-holesky")
    pub network: String,

    /// RPC URL for settlement layer
    pub rpc_url: Url,

    /// Ethereum-specific configuration (required if network is ethereum-*)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ethereum: Option<EthereumSettlementConfig>,

    /// Starknet-specific configuration (required if network is starknet-*)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starknet: Option<StarknetSettlementConfig>,
}

impl SettlementConfig {
    pub fn chain_type(&self, registry: &NetworkRegistry) -> Result<ChainType> {
        registry.chain_type(&self.network).ok_or_else(|| anyhow::anyhow!("Unknown network: {}", self.network))
    }

    pub fn is_ethereum(&self, registry: &NetworkRegistry) -> bool {
        matches!(self.chain_type(registry), Ok(ChainType::Ethereum))
    }

    pub fn is_starknet(&self, registry: &NetworkRegistry) -> bool {
        matches!(self.chain_type(registry), Ok(ChainType::Starknet))
    }

    /// Validate that the config matches the network type
    pub fn validate(&self, registry: &NetworkRegistry) -> Result<()> {
        let chain_type = self.chain_type(registry).context("Failed to determine chain type for settlement network")?;

        match chain_type {
            ChainType::Ethereum => {
                if self.ethereum.is_none() {
                    anyhow::bail!("Network '{}' is Ethereum-based, but ethereum config is missing", self.network);
                }
                if self.starknet.is_some() {
                    tracing::warn!(
                        "Starknet config present but network '{}' is Ethereum-based (will be ignored)",
                        self.network
                    );
                }
            }
            ChainType::Starknet => {
                if self.starknet.is_none() {
                    anyhow::bail!("Network '{}' is Starknet-based, but starknet config is missing", self.network);
                }
                if self.ethereum.is_some() {
                    tracing::warn!(
                        "Ethereum config present but network '{}' is Starknet-based (will be ignored)",
                        self.network
                    );
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthereumSettlementConfig {
    pub core_contract_address: String,
    pub operator_private_key: String,
    pub operator_address: String,

    #[serde(default = "default_finality_wait")]
    pub finality_retry_wait_seconds: u64,

    #[serde(default = "default_gas_multiplier")]
    pub max_gas_price_multiplier: f64,

    #[serde(default)]
    pub disable_peerdas: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarknetSettlementConfig {
    pub private_key: String,
    pub account_address: String,
    pub cairo_core_contract_address: String,

    #[serde(default = "default_starknet_finality_wait")]
    pub finality_retry_wait_seconds: u64,
}

fn default_finality_wait() -> u64 {
    60
}

fn default_starknet_finality_wait() -> u64 {
    120
}

fn default_gas_multiplier() -> f64 {
    1.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ethereum_settlement_validation() {
        let registry = NetworkRegistry::builtin();

        let config = SettlementConfig {
            network: "ethereum-sepolia".to_string(),
            rpc_url: "https://sepolia.infura.io".parse().unwrap(),
            ethereum: Some(EthereumSettlementConfig {
                core_contract_address: "0xabc".to_string(),
                operator_private_key: "0x123".to_string(),
                operator_address: "0xdef".to_string(),
                finality_retry_wait_seconds: 60,
                max_gas_price_multiplier: 1.5,
                disable_peerdas: false,
            }),
            starknet: None,
        };

        assert!(config.validate(&registry).is_ok());
        assert!(config.is_ethereum(&registry));
        assert!(!config.is_starknet(&registry));
    }

    #[test]
    fn test_starknet_settlement_validation() {
        let registry = NetworkRegistry::builtin();

        let config = SettlementConfig {
            network: "starknet-sepolia".to_string(),
            rpc_url: "https://starknet-sepolia.public.blastapi.io".parse().unwrap(),
            ethereum: None,
            starknet: Some(StarknetSettlementConfig {
                private_key: "0x123".to_string(),
                account_address: "0xabc".to_string(),
                cairo_core_contract_address: "0xdef".to_string(),
                finality_retry_wait_seconds: 120,
            }),
        };

        assert!(config.validate(&registry).is_ok());
        assert!(config.is_starknet(&registry));
        assert!(!config.is_ethereum(&registry));
    }

    #[test]
    fn test_missing_ethereum_config() {
        let registry = NetworkRegistry::builtin();

        let config = SettlementConfig {
            network: "ethereum-sepolia".to_string(),
            rpc_url: "https://sepolia.infura.io".parse().unwrap(),
            ethereum: None,
            starknet: None,
        };

        let result = config.validate(&registry);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("ethereum config is missing"));
    }

    #[test]
    fn test_unknown_network() {
        let registry = NetworkRegistry::builtin();

        let config = SettlementConfig {
            network: "unknown-network".to_string(),
            rpc_url: "https://example.com".parse().unwrap(),
            ethereum: None,
            starknet: None,
        };

        let result = config.chain_type(&registry);
        assert!(result.is_err());
    }
}
