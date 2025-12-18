use starknet_core::types::StarknetError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StarknetClientError {
    #[error("Starknet provider error: {0}")]
    Provider(String),

    #[error("Event processing error: {message} for event {event_id}")]
    EventProcessing { message: String, event_id: String },

    #[error("State sync error: {message} at block {block_number}")]
    StateSync { message: String, block_number: u64 },

    #[error("Contract interaction failed: {0}")]
    Contract(String),

    #[error("Value conversion error: {0}")]
    Conversion(String),

    #[error("Missing field in response: {0}")]
    MissingField(&'static str),

    #[error("Event subscription error: {message}")]
    EventSubscription { message: String },

    #[error("State initialization error: {message}")]
    StateInitialization { message: String },

    #[error("Network connection error: {message}")]
    NetworkConnection { message: String },

    #[error("Invalid response format: {message}")]
    InvalidResponseFormat { message: String },

    #[error("L1-L2 messaging error: {message}")]
    L1ToL2Messaging { message: String },

    #[error("Message processing error: {message}")]
    MessageProcessing { message: String },
}

impl From<StarknetError> for StarknetClientError {
    fn from(e: StarknetError) -> Self {
        StarknetClientError::Provider(e.to_string())
    }
}

impl StarknetClientError {
    /// Returns true if the error is recoverable (network/connection issues).
    /// These are transient errors that should be retried with backoff.
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Provider(_) | Self::EventSubscription { .. } | Self::NetworkConnection { .. })
    }
}
