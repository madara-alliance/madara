#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]
use alloy::rpc::client::RpcClient;
use alloy::transports::http::Http;
use async_trait::async_trait;
use color_eyre::Result;
use reqwest::Client;
use starknet::core::types::FieldElement;
use std::str::FromStr;
use url::Url;

use config::EthereumDaConfig;
use da_client_interface::{DaClient, DaVerificationStatus};

pub mod config;
pub struct EthereumDaClient {
    #[allow(dead_code)]
    provider: RpcClient<Http<Client>>,
}

#[async_trait]
impl DaClient for EthereumDaClient {
    async fn publish_state_diff(&self, _state_diff: Vec<FieldElement>) -> Result<String> {
        unimplemented!()
    }

    async fn verify_inclusion(&self, _external_id: &str) -> Result<DaVerificationStatus> {
        todo!()
    }
}

impl From<EthereumDaConfig> for EthereumDaClient {
    fn from(config: EthereumDaConfig) -> Self {
        let provider = RpcClient::builder()
            .reqwest_http(Url::from_str(config.rpc_url.as_str()).expect("Failed to parse ETHEREUM_RPC_URL"));
        EthereumDaClient { provider }
    }
}
