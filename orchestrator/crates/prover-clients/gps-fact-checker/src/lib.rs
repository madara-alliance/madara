use std::str::FromStr as _;

use alloy::primitives::{Address, B256};
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::sol;
use alloy::transports::http::{Client, Http};
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

pub struct FactChecker {
    fact_registry: FactRegistry::FactRegistryInstance<TransportT, ProviderT>,
}

type TransportT = Http<Client>;
type ProviderT = RootProvider<TransportT>;

impl FactChecker {
    pub fn new(sharp_rpc_node_url: Url, gps_verifier_contract_address: String) -> Self {
        let provider = ProviderBuilder::new().on_http(sharp_rpc_node_url);
        let gps_verifier_contract_address_str =
            format!("{:0>40}", &gps_verifier_contract_address.as_str().clone().trim_start_matches("0x"));
        let verifier_address =
            Address::from_str(&gps_verifier_contract_address_str).expect("Invalid GPS verifier contract address");

        let fact_registry = FactRegistry::new(verifier_address, provider);
        Self { fact_registry }
    }

    pub async fn is_valid(&self, fact: &B256) -> Result<bool, FactCheckerError> {
        let FactRegistry::isValidReturn { _0 } =
            self.fact_registry.isValid(*fact).call().await.map_err(FactCheckerError::InvalidFact)?;
        Ok(_0)
    }
}
