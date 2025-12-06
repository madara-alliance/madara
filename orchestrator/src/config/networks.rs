use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChainType {
    Ethereum,
    Starknet,
}

impl std::fmt::Display for ChainType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainType::Ethereum => write!(f, "ethereum"),
            ChainType::Starknet => write!(f, "starknet"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInfo {
    pub name: String,
    pub chain_type: ChainType,
    pub chain_id: String,
    pub is_testnet: bool,

    /// Default RPC URLs (fallback if not specified in config)
    #[serde(default)]
    pub default_rpc_urls: Vec<String>,
}

/// Registry of known networks
#[derive(Debug, Clone)]
pub struct NetworkRegistry {
    networks: HashMap<String, NetworkInfo>,
}

impl NetworkRegistry {
    /// Create a new registry with built-in networks
    pub fn builtin() -> Self {
        let mut networks = HashMap::new();

        // Ethereum networks
        networks.insert("ethereum-mainnet".to_string(), NetworkInfo {
            name: "ethereum-mainnet".to_string(),
            chain_type: ChainType::Ethereum,
            chain_id: "0x1".to_string(),
            is_testnet: false,
            default_rpc_urls: vec!["https://eth.llamarpc.com".to_string()],
        });

        networks.insert("ethereum-sepolia".to_string(), NetworkInfo {
            name: "ethereum-sepolia".to_string(),
            chain_type: ChainType::Ethereum,
            chain_id: "0xaa36a7".to_string(),
            is_testnet: true,
            default_rpc_urls: vec!["https://rpc.sepolia.org".to_string()],
        });

        networks.insert("ethereum-holesky".to_string(), NetworkInfo {
            name: "ethereum-holesky".to_string(),
            chain_type: ChainType::Ethereum,
            chain_id: "0x4268".to_string(),
            is_testnet: true,
            default_rpc_urls: vec!["https://holesky.drpc.org".to_string()],
        });

        // Starknet networks
        networks.insert("starknet-mainnet".to_string(), NetworkInfo {
            name: "starknet-mainnet".to_string(),
            chain_type: ChainType::Starknet,
            chain_id: "0x534e5f4d41494e".to_string(),
            is_testnet: false,
            default_rpc_urls: vec!["https://starknet-mainnet.public.blastapi.io".to_string()],
        });

        networks.insert("starknet-sepolia".to_string(), NetworkInfo {
            name: "starknet-sepolia".to_string(),
            chain_type: ChainType::Starknet,
            chain_id: "0x534e5f5345504f4c4941".to_string(),
            is_testnet: true,
            default_rpc_urls: vec!["https://starknet-sepolia.public.blastapi.io".to_string()],
        });

        // Local development network (Anvil)
        networks.insert("ethereum-local".to_string(), NetworkInfo {
            name: "ethereum-local".to_string(),
            chain_type: ChainType::Ethereum,
            chain_id: "0x7a69".to_string(), // Anvil default
            is_testnet: true,
            default_rpc_urls: vec!["http://localhost:8545".to_string()],
        });

        Self { networks }
    }

    /// Get network info by name
    pub fn get(&self, network: &str) -> Option<&NetworkInfo> {
        self.networks.get(network)
    }

    /// Get chain type for a network
    pub fn chain_type(&self, network: &str) -> Option<ChainType> {
        self.get(network).map(|info| info.chain_type)
    }

    /// Check if a network is registered
    pub fn contains(&self, network: &str) -> bool {
        self.networks.contains_key(network)
    }

    /// Register a custom network
    pub fn register_custom(&mut self, network: NetworkInfo) {
        self.networks.insert(network.name.clone(), network);
    }

    /// Register multiple custom networks
    pub fn register_custom_networks(&mut self, networks: Vec<NetworkInfo>) {
        for network in networks {
            self.register_custom(network);
        }
    }

    /// List all registered networks
    pub fn list_networks(&self) -> Vec<&NetworkInfo> {
        let mut networks: Vec<_> = self.networks.values().collect();
        networks.sort_by(|a, b| a.name.cmp(&b.name));
        networks
    }

    /// List networks by chain type
    pub fn list_by_chain_type(&self, chain_type: ChainType) -> Vec<&NetworkInfo> {
        let mut networks: Vec<_> = self.networks.values()
            .filter(|n| n.chain_type == chain_type)
            .collect();
        networks.sort_by(|a, b| a.name.cmp(&b.name));
        networks
    }
}

impl Default for NetworkRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_networks() {
        let registry = NetworkRegistry::builtin();

        // Check Ethereum networks
        assert!(registry.contains("ethereum-mainnet"));
        assert!(registry.contains("ethereum-sepolia"));
        assert!(registry.contains("ethereum-holesky"));

        // Check Starknet networks
        assert!(registry.contains("starknet-mainnet"));
        assert!(registry.contains("starknet-sepolia"));

        // Check local network
        assert!(registry.contains("ethereum-local"));
    }

    #[test]
    fn test_chain_type() {
        let registry = NetworkRegistry::builtin();

        assert_eq!(registry.chain_type("ethereum-sepolia"), Some(ChainType::Ethereum));
        assert_eq!(registry.chain_type("starknet-mainnet"), Some(ChainType::Starknet));
        assert_eq!(registry.chain_type("unknown"), None);
    }

    #[test]
    fn test_register_custom() {
        let mut registry = NetworkRegistry::builtin();

        let custom = NetworkInfo {
            name: "ethereum-custom".to_string(),
            chain_type: ChainType::Ethereum,
            chain_id: "0x12345".to_string(),
            is_testnet: true,
            default_rpc_urls: vec!["https://custom.example.com".to_string()],
        };

        registry.register_custom(custom);
        assert!(registry.contains("ethereum-custom"));
        assert_eq!(registry.chain_type("ethereum-custom"), Some(ChainType::Ethereum));
    }

    #[test]
    fn test_list_by_chain_type() {
        let registry = NetworkRegistry::builtin();

        let ethereum_networks = registry.list_by_chain_type(ChainType::Ethereum);
        assert!(ethereum_networks.len() >= 3);
        assert!(ethereum_networks.iter().all(|n| n.chain_type == ChainType::Ethereum));

        let starknet_networks = registry.list_by_chain_type(ChainType::Starknet);
        assert!(starknet_networks.len() >= 2);
        assert!(starknet_networks.iter().all(|n| n.chain_type == ChainType::Starknet));
    }
}
