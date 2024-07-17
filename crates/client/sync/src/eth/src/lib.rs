pub mod client;
pub mod utils;

pub mod config;
pub mod error;
use std::time::Duration;


use alloy::{
    network::{EthereumWallet, TransactionBuilder},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    transports::http::Http,

};

use reqwest::Client;

use crate::config::{ EthereumProviderConfig, EthereumWalletConfig};
use crate::error::Error;

impl TryFrom<EthereumProviderConfig> for dyn Provider<Http<Client>> {
    type Error = Error;

    fn try_from(config: EthereumProviderConfig) -> Result<Self, Self::Error> {
        match config {
            EthereumProviderConfig::Http(config) => {
                let wallet: EthereumWallet = config.wallet
                    .unwrap_or_default()
                    .try_into()
                    .map_err(|e| Error::WalletUnavailable(e))?;

                let rpc_endpoint = config.rpc_endpoint
                    .parse()
                    .map_err(|e: url::ParseError| Error::ProviderUrlParse(e))?;

                let provider = ProviderBuilder::new()
                    .with_recommended_fillers()
                    .wallet(wallet)
                    .on_http(rpc_endpoint);

                Ok(provider)
            }
        }
    }
}

impl TryFrom<EthereumWalletConfig> for EthereumWallet {
    type Error = Error;

    fn try_from(config: EthereumWalletConfig) -> Result<Self, Self::Error> {
        match config {
            EthereumWalletConfig::Local(config) => {
                let signer: PrivateKeySigner  = config.private_key
                    .parse()
                    .map_err(Error::PrivateKeyParse)?;
                Ok(EthereumWallet::from(signer))
            }
        }
    }
}

