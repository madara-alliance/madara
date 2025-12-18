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

    #[error("Value conversion error: {0}")]
    Conversion(String),

    #[error("Missing field in response: {0}")]
    MissingField(&'static str),

    #[error("Archive node required: {0}")]
    ArchiveRequired(String),

    #[error("Event stream error: {message}")]
    EventStream { message: String },

    #[error("State update error: {message}")]
    StateUpdate { message: String },

    #[error("Gas price calculation error: {message}")]
    GasPriceCalculation { message: String },

    #[error("L1 to L2 messaging error: {message}")]
    L1ToL2Messaging { message: String },

    #[error("Network connection error: {message}")]
    NetworkConnection { message: String },
}

impl From<sol_types::Error> for EthereumClientError {
    fn from(e: sol_types::Error) -> Self {
        EthereumClientError::Contract(e.to_string())
    }
}

impl EthereumClientError {
    /// Returns true if the error is recoverable (network/connection issues).
    /// These are transient errors that should be retried with backoff.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Rpc(_) | Self::EventStream { .. } | Self::NetworkConnection { .. })
    }
}
