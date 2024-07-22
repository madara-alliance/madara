#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use alloy::network::Ethereum;
use alloy::providers::RootProvider;
use alloy::transports::http::Http;
use async_trait::async_trait;
use color_eyre::Result;
use da_client_interface::{DaClient, DaVerificationStatus};
use mockall::automock;
use mockall::predicate::*;
use reqwest::Client;
pub mod config;
pub struct EthereumDaClient {
    #[allow(dead_code)]
    provider: RootProvider<Ethereum, Http<Client>>,
}

#[automock]
#[async_trait]
impl DaClient for EthereumDaClient {
    async fn publish_state_diff(&self, _state_diff: Vec<Vec<u8>>, _to: &[u8; 32]) -> Result<String> {
        // Here in case of ethereum we are not publishing the state diff because we are doing it all together in update_state job.
        // So we don't need to send the blob here.
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
