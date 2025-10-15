use thiserror::Error;

#[derive(Error, Debug)]
pub enum MadaraError {
    #[error("Account not initialized - must call init() first")]
    AccountNotInitialized,

    #[error("Class hash not found for contract: {0}")]
    ClassHashNotFound(String),

    #[error("Required field '{0}' not found in base layer addresses")]
    MissingBaseLayerAddress(String),

    #[error("Provider error: {0}")]
    ProviderError(#[from] starknet::providers::ProviderError),

    #[error("Anyhow error: {0}")]
    AnyhowError(#[from] anyhow::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Eth address parse error: {0}")]
    EthAddressParseError(#[from] starknet::core::types::eth_address::FromHexError),

    #[error("File error: {0}")]
    FileError(#[from] crate::utils::FileError),
}
