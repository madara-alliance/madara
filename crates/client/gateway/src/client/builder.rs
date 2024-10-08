use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client,
};
use url::Url;

#[derive(Debug, Clone)]
pub struct FeederClient {
    pub(crate) client: Client,
    #[allow(dead_code)]
    pub(crate) gateway_url: Url,
    pub(crate) feeder_gateway_url: Url,
    pub(crate) headers: HeaderMap,
}

impl FeederClient {
    pub fn new(gateway_url: Url, feeder_gateway_url: Url) -> Self {
        Self { client: Client::new(), gateway_url, feeder_gateway_url, headers: HeaderMap::new() }
    }

    pub fn new_with_headers(gateway_url: Url, feeder_gateway_url: Url, headers: &[(HeaderName, HeaderValue)]) -> Self {
        let headers = headers.iter().cloned().collect();
        Self { client: Client::new(), gateway_url, feeder_gateway_url, headers }
    }

    pub fn add_header(&mut self, name: HeaderName, value: HeaderValue) {
        self.headers.insert(name, value);
    }

    pub fn remove_header(&mut self, name: HeaderName) -> Option<HeaderValue> {
        self.headers.remove(name)
    }

    pub fn starknet_alpha_mainnet() -> Self {
        Self::new(
            Url::parse("https://alpha-mainnet.starknet.io/gateway/")
                .expect("Failed to parse Starknet Alpha Mainnet gateway url. This should not fail in prod."),
            Url::parse("https://alpha-mainnet.starknet.io/feeder_gateway/")
                .expect("Failed to parse Starknet Alpha Mainnet feeder gateway url. This should not fail in prod."),
        )
    }

    pub fn starknet_alpha_sepolia() -> Self {
        Self::new(
            Url::parse("https://alpha-sepolia.starknet.io/gateway/")
                .expect("Failed to parse Starknet Alpha Sepolia gateway url. This should not fail in prod."),
            Url::parse("https://alpha-sepolia.starknet.io/feeder_gateway/")
                .expect("Failed to parse Starknet Alpha Sepolia feeder gateway url. This should not fail in prod."),
        )
    }
}
