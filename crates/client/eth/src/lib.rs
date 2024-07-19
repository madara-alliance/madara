pub mod utils;

pub mod config;
pub mod error;

use alloy::providers::{Provider, ProviderBuilder, ReqwestProvider};

use crate::config::EthereumProviderConfig;
use crate::error::Error;

pub struct EthereumClient {
    #[allow(dead_code)]
    provider: ReqwestProvider,
}
impl TryFrom<EthereumProviderConfig> for EthereumClient {
    type Error = Error;

    fn try_from(config: EthereumProviderConfig) -> Result<Self, Self::Error> {
        match config {
            EthereumProviderConfig::Http(config) => {
                let rpc_endpoint =
                    config.rpc_endpoint.parse().map_err(|e: url::ParseError| Error::ProviderUrlParse(e))?;
                // let client =
                //     RpcClient::new_http(rpc_endpoint);
                // let provider = ProviderBuilder::<_, Ethereum>::new().on_client(client);
                let provider = ProviderBuilder::new().on_http(rpc_endpoint);

                Ok(EthereumClient { provider })
            }
        }
    }
}
