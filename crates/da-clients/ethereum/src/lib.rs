#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::str::FromStr;

use alloy::network::Ethereum;
use alloy::providers::{ProviderBuilder, RootProvider};
use alloy::rpc::client::RpcClient;
use alloy::transports::http::Http;
use async_trait::async_trait;
use color_eyre::Result;
use da_client_interface::{DaClient, DaVerificationStatus};
use mockall::automock;
use mockall::predicate::*;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EthereumDaValidatedArgs {
    pub ethereum_da_rpc_url: Url,
}

pub struct EthereumDaClient {
    #[allow(dead_code)]
    provider: RootProvider<Ethereum, Http<Client>>,
}

impl EthereumDaClient {
    pub async fn new_with_args(ethereum_da_params: &EthereumDaValidatedArgs) -> Self {
        let client = RpcClient::new_http(
            Url::from_str(ethereum_da_params.ethereum_da_rpc_url.as_str())
                .expect("Failed to parse ethereum_da_rpc_url"),
        );
        let provider = ProviderBuilder::<_, Ethereum>::new().on_client(client);
        Self { provider }
    }
}

#[automock]
#[async_trait]
impl DaClient for EthereumDaClient {
    async fn publish_state_diff(&self, _state_diff: Vec<Vec<u8>>, _to: &[u8; 32]) -> Result<String> {
        // Here in case of ethereum we are not publishing the state diff because we are doing it all
        // together in update_state job. So we don't need to send the blob here.
        Ok("NA".to_string())
    }

    async fn verify_inclusion(&self, _external_id: &str) -> Result<DaVerificationStatus> {
        Ok(DaVerificationStatus::Verified)
    }

    async fn max_blob_per_txn(&self) -> u64 {
        6
    }

    async fn max_bytes_per_blob(&self) -> u64 {
        131072
    }
}
