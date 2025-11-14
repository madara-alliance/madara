#![allow(missing_docs)]
#![allow(clippy::missing_docs_in_private_items)]

use std::sync::Arc;

use alloy::providers::ProviderBuilder;
use async_trait::async_trait;
use color_eyre::Result;
use mockall::automock;
use mockall::predicate::*;
use orchestrator_da_client_interface::{DaClient, DaVerificationStatus};
use serde::{Deserialize, Serialize};
use url::Url;

pub type DefaultHttpProvider = alloy::providers::fillers::FillProvider<
    alloy::providers::fillers::JoinFill<
        alloy::providers::Identity,
        alloy::providers::fillers::JoinFill<
            alloy::providers::fillers::GasFiller,
            alloy::providers::fillers::JoinFill<
                alloy::providers::fillers::BlobGasFiller,
                alloy::providers::fillers::JoinFill<
                    alloy::providers::fillers::NonceFiller,
                    alloy::providers::fillers::ChainIdFiller,
                >,
            >,
        >,
    >,
    alloy::providers::RootProvider<alloy::network::Ethereum>,
    alloy::network::Ethereum,
>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EthereumDaValidatedArgs {
    pub ethereum_da_rpc_url: Url,
}

pub struct EthereumDaClient {
    #[allow(dead_code)]
    provider: Arc<DefaultHttpProvider>,
}

impl EthereumDaClient {
    pub async fn new_with_args(args: &EthereumDaValidatedArgs) -> Self {
        let provider = Arc::new(ProviderBuilder::new().connect_http(args.ethereum_da_rpc_url.clone()));
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
