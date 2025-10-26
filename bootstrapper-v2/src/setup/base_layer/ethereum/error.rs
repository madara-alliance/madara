use std::io::Error as IoError;
use thiserror::Error;

use crate::{
    setup::base_layer::{ethereum::factory::FactorySetupError, BaseLayerError},
    utils::FileError,
};

#[derive(Error, Debug)]
pub enum EthereumError {
    #[error("Ethereum file error: {0}")]
    File(#[from] IoError),

    #[error("The key specified does not exist {0}")]
    KeyDoesNotExist(String),

    #[error("Invalid hex value does not start with 0x")]
    InvalidHexValue,

    #[error("Alloy rpc error: {0}")]
    AlloyRpcError(#[from] alloy::transports::RpcError<alloy::transports::TransportErrorKind>),

    #[error("Contract Address not found in the reciept")]
    MissingContractInReceipt,

    #[error("Key {key} not found in map, even after explicit check")]
    KeyNotFound { key: String },

    #[error("Serde error: {0}")]
    SerdeError(#[from] serde_json::Error),

    #[error("Faile to parse rpc url: {0}")]
    RpcUrlParseError(String),

    #[error("Failed to save output: {0}")]
    FailedToSaveOutput(#[from] FileError),

    #[error("Deployment of contracts from factory while calling setup failed: {0}")]
    FactorySetupFailed(#[from] FactorySetupError),

    #[error("Failed to parse alloy type: {0}")]
    AddressParseError(#[from] alloy::primitives::ruint::ParseError),

    #[error("Ethereum anyhow error: {0}")]
    AnyhowError(#[from] anyhow::Error),

    #[error("Ethereum hex error: {0}")]
    HexError(#[from] alloy::hex::FromHexError),

    #[error("Failed to hex decode ethereum bytecode")]
    FailedToHexDecode(#[from] hex::FromHexError),

    #[error("Alloy provider error: {0}")]
    AlloyPendingTransactionError(#[from] alloy::providers::PendingTransactionError),

    #[error("Alloy Contract error: {0}")]
    AlloyContractError(#[from] alloy::contract::Error),

    #[error("Other error: {0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}

impl From<EthereumError> for BaseLayerError {
    fn from(value: EthereumError) -> Self {
        Self::Internal(Box::new(value))
    }
}
