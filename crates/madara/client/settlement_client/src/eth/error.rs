use thiserror::Error;
use alloy::sol_types;
use crate::error::SettlementClientError;

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
}

impl From<sol_types::Error> for EthereumClientError {
    fn from(e: sol_types::Error) -> Self {
        EthereumClientError::Contract(e.to_string())
    }
}

// impl From<EthereumClientError> for SettlementClientError {
//     fn from(err: EthereumClientError) -> Self {
//         SettlementClientError::Ethereum(err)
//     }
// }
