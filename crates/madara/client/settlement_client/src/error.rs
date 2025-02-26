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

    #[error("{0}")]
    Other(String),

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

// Updated ResultExt to be more specific
pub trait ResultExt<T, E> {
    fn settlement_context<C>(self, context: C) -> Result<T, SettlementClientError>
    where
        C: Into<String>;
}

// Single implementation that handles both std::error::Error and String
impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: std::fmt::Display,  // Changed from std::error::Error to std::fmt::Display
{
    fn settlement_context<C>(self, context: C) -> Result<T, SettlementClientError>
    where
        C: Into<String>,
    {
        self.map_err(|e| SettlementClientError::Other(format!("{}: {}", context.into(), e)))
    }
}

// // Add this implementation specifically for anyhow::Error
// impl<T> ResultExt<T, anyhow::Error> for Result<T, anyhow::Error> {
//     fn settlement_context<C>(self, context: C) -> Result<T, SettlementClientError>
//     where
//         C: Into<String>,
//     {
//         self.map_err(|e| SettlementClientError::Other(format!("{}: {}", context.into(), e)))
//     }
// }
