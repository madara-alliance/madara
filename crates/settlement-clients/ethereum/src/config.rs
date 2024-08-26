use std::str::FromStr;

use serde::{Deserialize, Serialize};
use settlement_client_interface::SettlementConfig;
use url::Url;
use utils::env_utils::get_env_var_or_panic;

pub const ENV_CORE_CONTRACT_ADDRESS: &str = "STARKNET_SOLIDITY_CORE_CONTRACT_ADDRESS";
pub const SETTLEMENT_RPC_URL: &str = "SETTLEMENT_RPC_URL";
pub const L1_CORE_CONTRACT_ADDRESS: &str = "L1_CORE_CONTRACT_ADDRESS";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthereumSettlementConfig {
    pub rpc_url: Url,
    pub core_contract_address: String,
}

impl SettlementConfig for EthereumSettlementConfig {
    fn new_from_env() -> Self {
        let rpc_url = get_env_var_or_panic(SETTLEMENT_RPC_URL);
        let rpc_url = Url::from_str(&rpc_url).unwrap_or_else(|_| panic!("Failed to parse {}", SETTLEMENT_RPC_URL));
        let core_contract_address = get_env_var_or_panic(ENV_CORE_CONTRACT_ADDRESS);
        Self { rpc_url, core_contract_address }
    }
}

impl Default for EthereumSettlementConfig {
    fn default() -> Self {
        Self {
            rpc_url: get_env_var_or_panic(SETTLEMENT_RPC_URL).parse().unwrap(),
            core_contract_address: get_env_var_or_panic(L1_CORE_CONTRACT_ADDRESS),
        }
    }
}
