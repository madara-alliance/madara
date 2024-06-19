pub mod error;
pub mod fact_info;
pub mod fact_node;
pub mod fact_topology;

use alloy::primitives::{Address, B256};
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::sol;
use alloy::transports::http::{Client, Http};
use url::Url;

use self::error::FactCheckerError;

sol!(
    #[allow(missing_docs)]
    #[sol(rpc)]
    FactRegistry,
    "tests/artifacts/FactRegistry.json"
);

pub struct FactChecker {
    fact_registry: FactRegistry::FactRegistryInstance<TransportT, ProviderT>,
}

type TransportT = Http<Client>;
type ProviderT = RootProvider<TransportT>;

impl FactChecker {
    pub fn new(rpc_node_url: Url, verifier_address: Address) -> Self {
        let provider = ProviderBuilder::new().on_http(rpc_node_url);
        let fact_registry = FactRegistry::new(verifier_address, provider);
        Self { fact_registry }
    }

    pub async fn is_valid(&self, fact: &B256) -> Result<bool, FactCheckerError> {
        let FactRegistry::isValidReturn { _0 } =
            self.fact_registry.isValid(*fact).call().await.map_err(FactCheckerError::FactRegistry)?;
        Ok(_0)
    }
}
