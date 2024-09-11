use crate::EthereumDaClient;
use alloy::network::Ethereum;
use alloy::providers::ProviderBuilder;
use alloy::rpc::client::RpcClient;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use url::Url;
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
            rpc_url: settings.get_settings_or_panic("SETTLEMENT_RPC_URL"),
            memory_pages_contract: settings.get_settings_or_panic("MEMORY_PAGES_CONTRACT_ADDRESS"),
            private_key: settings.get_settings_or_panic("PRIVATE_KEY"),
        })
    }

    pub async fn build_client(&self) -> EthereumDaClient {
        let client =
            RpcClient::new_http(Url::from_str(self.rpc_url.as_str()).expect("Failed to parse SETTLEMENT_RPC_URL"));
        let provider = ProviderBuilder::<_, Ethereum>::new().on_client(client);

        EthereumDaClient { provider }
    }
}
