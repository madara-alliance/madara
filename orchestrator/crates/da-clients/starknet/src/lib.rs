#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::Result;
use mockall::predicate::*;
use orchestrator_da_client_interface::{DaClient, DaVerificationStatus};
use serde::{Deserialize, Serialize};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::JsonRpcClient;
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StarknetDaValidatedArgs {
    pub starknet_da_rpc_url: Url,
}

pub struct StarknetDaClient {
    #[allow(dead_code)]
    provider: Arc<JsonRpcClient<HttpTransport>>,
}

impl StarknetDaClient {
    pub async fn new_with_args(starknet_da_params: &StarknetDaValidatedArgs) -> Self {
        let client = JsonRpcClient::new(HttpTransport::new(
            Url::from_str(starknet_da_params.starknet_da_rpc_url.as_str())
                .inspect_err(|e| {
                    tracing::error!("Failed to parse Starknet DA RPC URL: {}", e);
                })
                .expect("invalid url provided"),
        ));
        Self { provider: Arc::new(client) }
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
impl DaClient for StarknetDaClient {
    async fn publish_state_diff(&self, _state_diff: Vec<Vec<u8>>, _to: &[u8; 32]) -> Result<String> {
        // Here in case of starknet we are not publishing the state diff because we are doing it all
        // together in proving and update_state job. So we don't need to send anything here.
        Ok("NA".to_string())
    }

    async fn verify_inclusion(&self, _external_id: &str) -> Result<DaVerificationStatus> {
        Ok(DaVerificationStatus::Verified)
    }

    async fn max_blob_per_txn(&self) -> u64 {
        6
    }

    // max_bytes_per_blob - return's the maximum size of a blob in Starknet
    // Maximum size of a blob in Starknet is 128KB (131072 bytes)
    async fn max_bytes_per_blob(&self) -> u64 {
        131072
    }
}
