use crate::eth::error::EthereumClientError;
use crate::starknet::error::StarknetClientError;
use anyhow::Context;
use thiserror::Error;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to parse provider URL: {0}")]
    ProviderUrlParse(#[from] url::ParseError),
    #[error("Failed to parse contract address: {0}")]
    AddressParseError(#[from] alloy::primitives::AddressError),
    #[error("Failed to read config from file: {0}")]
    ConfigReadFromFile(#[from] std::io::Error),
    #[error("Failed to decode from JSON: {0}")]
    ConfigDecodeFromJson(#[from] serde_json::Error),
}

#[derive(Error, Debug)]
pub enum SettlementClientError {
    #[error("Ethereum client error: {0}")]
    Ethereum(#[from] EthereumClientError),

    #[error("Starknet client error: {0}")]
    Starknet(#[from] StarknetClientError),

    #[error(transparent)]
    Other(#[from] anyhow::Error),

    #[error("Missing required field: {0}")]
    MissingField(&'static str),

    #[error("Value exceeds Felt max value for field: {0}")]
    ValueExceedsFeltRange(&'static str),

    #[error("Invalid log: {0}")]
    InvalidLog(String),

    #[error("Invalid contract: {0}")]
    InvalidContract(String),

    #[error("Conversion error: {0}")]
    ConversionError(String),

    #[error("Invalid event: {0}")]
    InvalidEvent(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),
}

// Helper trait for error context
pub trait ResultExt<T> {
    fn with_context<C, F>(self, context: F) -> Result<T, SettlementClientError>
    where
        F: FnOnce() -> C,
        C: std::fmt::Display + Send + Sync + 'static;
}

impl<T, E> ResultExt<T> for Result<T, E>
where
    E: Into<SettlementClientError>,
{
    fn with_context<C, F>(self, context: F) -> Result<T, SettlementClientError>
    where
        F: FnOnce() -> C,
        C: std::fmt::Display + Send + Sync + 'static,
    {
        self.map_err(|e| {
            let err: SettlementClientError = e.into();
            err.context(context()).into()
        })
    }
}
