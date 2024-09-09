use alloy::primitives::Address;
use serde::{Deserialize, Serialize};
use url::Url;
use utils::settings::Settings;

/// SHARP proving service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharpConfig {
    /// SHARP service url
    pub service_url: Url,
    /// EVM RPC node url
    pub rpc_node_url: Url,
    /// GPS verifier contract address (implements FactRegistry)
    pub verifier_address: Address,
}

impl SharpConfig {
    pub fn new_with_settings(settings: &impl Settings) -> color_eyre::Result<Self> {
        Ok(Self {
            service_url: settings.get_settings("SHARP_URL")?.parse().unwrap(),
            rpc_node_url: settings.get_settings("SETTLEMENT_RPC_URL")?.parse().unwrap(),
            verifier_address: settings.get_settings("MEMORY_PAGES_CONTRACT_ADDRESS")?.parse().unwrap(),
        })
    }
}
