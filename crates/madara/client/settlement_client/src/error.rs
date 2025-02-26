use crate::eth::error::EthereumClientError;
use crate::starknet::error::StarknetClientError;
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
    Ethereum(EthereumClientError),

    #[error("Starknet client error: {0}")]
    Starknet(StarknetClientError),

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

// 1. Ensure EthereumClientError can be converted to SettlementClientError
impl From<EthereumClientError> for SettlementClientError {
    fn from(err: EthereumClientError) -> Self {
        SettlementClientError::Ethereum(err)
    }
}

// 2. Ensure StarknetClientError can be converted to SettlementClientError
impl From<StarknetClientError> for SettlementClientError {
    fn from(err: StarknetClientError) -> Self {
        SettlementClientError::Starknet(err)
    }
}

// 3. Update ResultExt to be more flexible with error types
pub trait ResultExt<T, E> {
    fn settlement_context<C>(self, context: C) -> Result<T, SettlementClientError>
    where
        C: Into<String>;
}

// 4. Implementation for any error type that can be converted to SettlementClientError
impl<T, E> ResultExt<T, E> for Result<T, E>
where
    E: Into<SettlementClientError>,
{
    fn settlement_context<C>(self, context: C) -> Result<T, SettlementClientError>
    where
        C: Into<String>,
    {
        self.map_err(|e| {
            let err: SettlementClientError = e.into();
            SettlementClientError::Other(format!("{}: {}", context.into(), err))
        })
    }
}
