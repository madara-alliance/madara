use crate::error::SettlementClientError;
use alloy::sol_types;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EthereumClientError {
    #[error("Ethereum RPC error: {0}")]
    Rpc(String),

    #[error("Contract interaction failed: {0}")]
    Contract(String),

    #[error("Event processing error: {message} at block {block_number}")]
    EventProcessing { message: String, block_number: u64 },

    #[error("State sync error: {message} at block {block_number}")]
    StateSync { message: String, block_number: u64 },

    #[error("Value conversion error: {0}")]
    Conversion(String),

    #[error("Missing field in response: {0}")]
    MissingField(&'static str),

    #[error("Archive node required: {0}")]
    ArchiveRequired(String),

    #[error("Other error: {0}")]
    Other(String),

    #[error("Settlement error: {0}")]
    Settlement(String),
}

impl From<sol_types::Error> for EthereumClientError {
    fn from(e: sol_types::Error) -> Self {
        EthereumClientError::Contract(e.to_string())
    }
}

impl From<SettlementClientError> for EthereumClientError {
    fn from(error: SettlementClientError) -> Self {
        EthereumClientError::Settlement(error.to_string())
    }
}
