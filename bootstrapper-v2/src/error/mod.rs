pub mod madara;

use thiserror::Error;

use crate::setup::base_layer::BaseLayerError;
use crate::utils::FileError;

/// Result type for bootstrapper operations
pub type BootstrapperResult<T> = Result<T, BootstrapperError>;

/// Configuration validation error
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("l1_token_address is required when deploy_test_contracts is false")]
    MissingL1TokenAddress,
}

/// Main error enum for the bootstrapper
#[derive(Error, Debug)]
pub enum BootstrapperError {
    // Configuration errors
    #[error("Configuration error: {0}")]
    ConfigError(#[from] ConfigError),

    // Dotenvy errors
    #[error("Dotenvy error: {0}")]
    DotenvyError(#[from] dotenvy::Error),

    // File I/O errors
    #[error("File I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("File error: {0}")]
    FileError(#[from] FileError),

    #[error("Base layer init: {0}")]
    BaseLayerError(#[from] BaseLayerError),

    // Madara setup errors
    #[error("Madara setup error: {0}")]
    MadaraError(#[from] madara::MadaraError),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    // Additional external error conversions
    #[error("Provider error: {0}")]
    ProviderError(#[from] starknet::providers::ProviderError),

    #[error("Eth address parse error: {0}")]
    EthAddressParseError(#[from] starknet::core::types::eth_address::FromHexError),

    #[error("Other error: {0}")]
    Other(#[from] Box<dyn std::error::Error + Send + Sync>),
}
