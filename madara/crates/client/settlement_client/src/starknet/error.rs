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
}

impl From<StarknetError> for StarknetClientError {
    fn from(e: StarknetError) -> Self {
        StarknetClientError::Provider(e.to_string())
    }
}
