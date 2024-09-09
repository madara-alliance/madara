use serde::{Deserialize, Serialize};
use utils::settings::Settings;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EthereumDaConfig {
    pub rpc_url: String,
    pub memory_pages_contract: String,
    pub private_key: String,
}

impl EthereumDaConfig {
    pub fn new_with_settings(settings: &impl Settings) -> color_eyre::Result<Self> {
        Ok(Self {
            rpc_url: settings.get_settings("SETTLEMENT_RPC_URL")?,
            memory_pages_contract: settings.get_settings("MEMORY_PAGES_CONTRACT_ADDRESS")?,
            private_key: settings.get_settings("PRIVATE_KEY")?,
        })
    }
}
