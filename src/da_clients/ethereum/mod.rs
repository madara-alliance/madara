use alloy::rpc::client::RpcClient;
use alloy::transports::http::Http;
use async_trait::async_trait;
use color_eyre::Result;
use reqwest::Client;
use starknet::core::types::FieldElement;
use std::str::FromStr;
use url::Url;

use crate::da_clients::ethereum::config::EthereumDaConfig;
use crate::da_clients::DaClient;
use crate::jobs::types::JobVerificationStatus;

pub mod config;
pub struct EthereumDaClient {
    provider: RpcClient<Http<Client>>,
}

#[async_trait]
impl DaClient for EthereumDaClient {
    async fn publish_state_diff(&self, state_diff: Vec<FieldElement>) -> Result<String> {
        unimplemented!()
    }

    async fn verify_inclusion(&self, external_id: &String) -> Result<JobVerificationStatus> {
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
