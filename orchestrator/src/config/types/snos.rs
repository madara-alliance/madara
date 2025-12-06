use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SNOSConfig {
    pub rpc_url: Url,

    #[serde(default)]
    pub full_output: bool,

    #[serde(default = "default_strk_fee_token")]
    pub strk_fee_token_address: String,

    #[serde(default = "default_eth_fee_token")]
    pub eth_fee_token_address: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub versioned_constants_path: Option<std::path::PathBuf>,
}

fn default_strk_fee_token() -> String {
    "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d".to_string()
}

fn default_eth_fee_token() -> String {
    "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7".to_string()
}
