use std::collections::HashMap;

use reqwest::Client;
use url::Url;

#[derive(Debug, Clone)]
pub struct FeederClient {
    pub(crate) client: Client,
    pub(crate) gateway_url: Url,
    pub(crate) feeder_gateway_url: Url,
    pub(crate) headers: HashMap<String, String>,
}

impl FeederClient {
    pub fn new(gateway_url: Url, feeder_gateway_url: Url) -> Self {
        Self { client: Client::new(), gateway_url, feeder_gateway_url, headers: HashMap::new() }
    }

    pub fn new_with_headers(gateway_url: Url, feeder_gateway_url: Url, headers: &[(String, String)]) -> Self {
        let headers = headers.iter().cloned().collect();
        Self { client: Client::new(), gateway_url, feeder_gateway_url, headers }
    }

    pub fn add_header(&mut self, key: &str, value: &str) {
        self.headers.insert(key.to_string(), value.to_string());
    }

    pub fn remove_header(&mut self, key: &str) -> Option<String> {
        self.headers.remove(key)
    }

    pub fn starknet_alpha_mainnet() -> Self {
        Self::new(
            Url::parse("https://alpha-mainnet.starknet.io/gateway/").unwrap(),
            Url::parse("https://alpha-mainnet.starknet.io/feeder_gateway/").unwrap(),
        )
    }

    pub fn starknet_alpha_sepolia() -> Self {
        Self::new(
            Url::parse("https://alpha-sepolia.starknet.io/gateway/").unwrap(),
            Url::parse("https://alpha-sepolia.starknet.io/feeder_gateway/").unwrap(),
        )
    }
}
