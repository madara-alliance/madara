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
            service_url: settings
                .get_settings_or_panic("SHARP_URL")
                .parse()
                .expect("Failed to parse to service url for SharpConfig"),
            rpc_node_url: settings
                .get_settings_or_panic("SETTLEMENT_RPC_URL")
                .parse()
                .expect("Failed to parse to rpc_node_url for SharpConfig"),
            verifier_address: settings
                .get_settings_or_panic("GPS_VERIFIER_CONTRACT_ADDRESS")
                .parse()
                .expect("Failed to parse to verifier_address for SharpConfig"),
        })
    }
}
