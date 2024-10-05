use std::str::FromStr;

use serde::{Deserialize, Serialize};
use url::Url;
use utils::settings::Settings;

pub const SETTLEMENT_RPC_URL: &str = "SETTLEMENT_RPC_URL";
pub const L1_CORE_CONTRACT_ADDRESS: &str = "L1_CORE_CONTRACT_ADDRESS";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthereumSettlementConfig {
    pub rpc_url: Url,
    pub core_contract_address: String,
}

impl EthereumSettlementConfig {
    pub fn new_with_settings(settings: &impl Settings) -> Self {
        let rpc_url = settings.get_settings_or_panic(SETTLEMENT_RPC_URL);
        let rpc_url = Url::from_str(&rpc_url).unwrap_or_else(|_| panic!("Failed to parse {}", SETTLEMENT_RPC_URL));
        let core_contract_address = settings.get_settings_or_panic(L1_CORE_CONTRACT_ADDRESS);
        Self { rpc_url, core_contract_address }
    }
}
