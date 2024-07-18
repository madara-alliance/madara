pub mod utils;

pub mod config;
pub mod error;


use alloy::{
    network::{TransactionBuilder},
    providers::{Provider, ProviderBuilder},
    transports::http::Http,

};

use reqwest::Client;

use crate::config::{ EthereumProviderConfig};
use crate::error::Error;

impl TryFrom<EthereumProviderConfig> for dyn Provider<Http<Client>> {
    type Error = Error;

    fn try_from(config: EthereumProviderConfig) -> Result<Self, Self::Error> {
        match config {
            EthereumProviderConfig::Http(config) => {

                let rpc_endpoint = config.rpc_endpoint
                    .parse()
                    .map_err(|e: url::ParseError| Error::ProviderUrlParse(e))?;

                let provider = ProviderBuilder::new()
                    .with_recommended_fillers()
                    .on_http(rpc_endpoint);

                Ok(provider)
            }
        }
    }
}

