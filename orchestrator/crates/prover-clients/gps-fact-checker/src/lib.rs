use alloy::network::Ethereum;
use alloy::primitives::{Address, B256};
use alloy::providers::fillers::{BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller};
use alloy::providers::{Identity, ProviderBuilder, RootProvider};
use alloy::sol;
use std::str::FromStr;
use url::Url;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    FactRegistry,
    "tests/artifacts/FactRegistry.json"
);

#[derive(Debug, thiserror::Error)]
pub enum FactCheckerError {
    #[error("Fact registry call failed: {0}")]
    InvalidFact(#[source] alloy::contract::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettlementLayer {
    Ethereum,
    Starknet,
}

impl FromStr for SettlementLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "ethereum" => Ok(SettlementLayer::Ethereum),
            "starknet" => Ok(SettlementLayer::Starknet),
            _ => Err(format!("Unknown settlement layer: {}", s)),
        }
    }
}

pub struct FactChecker {
    fact_registry: Option<FactRegistry::FactRegistryInstance<ProviderT>>,
    settlement_layer: SettlementLayer,
}

// Type alias for the provider returned by connect_http (with default fillers)
type ProviderT = FillProvider<
    JoinFill<Identity, JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>>,
    RootProvider<Ethereum>,
    Ethereum,
>;

impl FactChecker {
    pub fn new(rpc_url: Url, gps_verifier_contract_address: String, settlement_layer: String) -> Self {
        let settlement_layer = SettlementLayer::from_str(&settlement_layer).expect("Invalid settlement layer");

        match settlement_layer {
            SettlementLayer::Ethereum => {
                let provider = ProviderBuilder::new().connect_http(rpc_url);
                let fact_registry = FactRegistry::new(
                    Address::from_str(gps_verifier_contract_address.as_str())
                        .expect("Invalid GPS verifier contract address"),
                    provider,
                );
                Self { fact_registry: Some(fact_registry), settlement_layer }
            }
            SettlementLayer::Starknet => Self { fact_registry: None, settlement_layer },
        }
    }

    pub async fn is_valid(&self, fact: &B256) -> Result<bool, FactCheckerError> {
        match self.settlement_layer {
            SettlementLayer::Ethereum => {
                let fact_registry =
                    self.fact_registry.as_ref().expect("Fact registry should be initialized for Ethereum");
                fact_registry.isValid(*fact).call().await.map_err(FactCheckerError::InvalidFact)
            }
            SettlementLayer::Starknet => {
                // TODO:L3 Implement actual Starknet fact checking
                // For now, return true as a mock implementation
                Ok(true)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settlement_layer_from_str() {
        assert_eq!(SettlementLayer::from_str("ethereum").unwrap(), SettlementLayer::Ethereum);
        assert_eq!(SettlementLayer::from_str("starknet").unwrap(), SettlementLayer::Starknet);
        assert!(SettlementLayer::from_str("invalid").is_err());
    }
}
