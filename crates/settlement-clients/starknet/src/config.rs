use std::str::FromStr;

use serde::{Deserialize, Serialize};
use settlement_client_interface::SettlementConfig;
use url::Url;
use utils::env_utils::{get_env_var_or_default, get_env_var_or_panic};

pub const ENV_STARKNET_RPC_URL: &str = "STARKNET_RPC_URL";
pub const ENV_CORE_CONTRACT_ADDRESS: &str = "STARKNET_CAIRO_CORE_CONTRACT_ADDRESS";

pub const ENV_STARKNET_FINALITY_RETRY_DELAY_IN_SECS: &str = "STARKNET_FINALITY_RETRY_WAIT_IN_SECS";
pub const DEFAULT_FINALITY_RETRY_DELAY: &str = "60";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarknetSettlementConfig {
    pub rpc_url: Url,
    pub core_contract_address: String,
    pub tx_finality_retry_delay_in_seconds: u64,
}

impl SettlementConfig for StarknetSettlementConfig {
    /// Should create a new instance of the DaConfig from the environment variables
    fn new_from_env() -> Self {
        let rpc_url = get_env_var_or_panic(ENV_STARKNET_RPC_URL);
        let rpc_url = Url::from_str(&rpc_url).unwrap_or_else(|_| panic!("Failed to parse {}", ENV_STARKNET_RPC_URL));
        let core_contract_address = get_env_var_or_panic(ENV_CORE_CONTRACT_ADDRESS);
        let tx_finality_retry_delay_in_seconds: u64 =
            get_env_var_or_default(ENV_STARKNET_FINALITY_RETRY_DELAY_IN_SECS, DEFAULT_FINALITY_RETRY_DELAY)
                .parse()
                .expect("STARKNET_FINALITY_RETRY_WAIT_IN_SECS should be a delay in seconds");
        StarknetSettlementConfig { rpc_url, core_contract_address, tx_finality_retry_delay_in_seconds }
    }
}

impl Default for StarknetSettlementConfig {
    fn default() -> Self {
        Self {
            rpc_url: "https://free-rpc.nethermind.io/sepolia-juno".parse().unwrap(),
            core_contract_address: "TODO:https://github.com/keep-starknet-strange/piltover".into(),
            tx_finality_retry_delay_in_seconds: 60,
        }
    }
}
