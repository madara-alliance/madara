use thiserror::Error;
use starknet_core::types::StarknetError;
use crate::error::SettlementClientError;


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

// impl From<StarknetClientError> for SettlementClientError {
//     fn from(err: StarknetClientError) -> Self {
//         SettlementClientError::Starknet(err)
//     }
// }

// impl From<StarknetClientError> for crate::error::SettlementClientError {
//     fn from(err: StarknetClientError) -> Self {
//         match err {
//             StarknetClientError::Provider(msg) => Self::InvalidResponse(msg),
//             StarknetClientError::EventProcessing { message, event_id } => 
//                 Self::InvalidEvent(format!("event {}: {}", event_id, message)),
//             StarknetClientError::StateSync { message, block_number } => 
//                 Self::InvalidLog(format!("at block {}: {}", block_number, message)),
//             StarknetClientError::Contract(msg) => Self::InvalidContract(msg),
//             StarknetClientError::Conversion(msg) => Self::ConversionError(msg),
//             StarknetClientError::MissingField(field) => Self::MissingField(field),
//         }
//     }
// }
