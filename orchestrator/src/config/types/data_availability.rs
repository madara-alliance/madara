use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataAvailabilityConfig {
    /// Network name for DA layer
    pub network: String,

    /// RPC URL (optional - can reuse settlement RPC if null and networks match)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpc_url: Option<Url>,
}

impl DataAvailabilityConfig {
    /// Resolve RPC URL, using settlement RPC if not specified
    pub fn resolve_rpc_url(&mut self, settlement_network: &str, settlement_rpc: &Url) {
        if self.rpc_url.is_none() && self.network == settlement_network {
            tracing::info!(
                "DA network matches settlement network ({}), reusing settlement RPC: {}",
                settlement_network,
                settlement_rpc
            );
            self.rpc_url = Some(settlement_rpc.clone());
        }
    }

    pub fn get_rpc_url(&self) -> Option<&Url> {
        self.rpc_url.as_ref()
    }
}
