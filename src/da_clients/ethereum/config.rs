use crate::da_clients::DaConfig;
use crate::utils::env_utils::get_env_var_or_panic;
use alloy::rpc::client::RpcClient;
use alloy::transports::http::Http;
use reqwest::Client;
use std::str::FromStr;
use url::Url;

#[derive(Clone, Debug)]
pub struct EthereumDaConfig {
    pub rpc_url: String,
    pub memory_pages_contract: String,
}

impl DaConfig for EthereumDaConfig {
    fn new_from_env() -> Self {
        Self {
            rpc_url: get_env_var_or_panic("ETHEREUM_RPC_URL"),
            memory_pages_contract: get_env_var_or_panic("MEMORY_PAGES_CONTRACT_ADDRESS"),
        }
    }
}
