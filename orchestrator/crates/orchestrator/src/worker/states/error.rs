use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum StateHandlerError {
    #[error("State handler for state {0} not found")]
    InvalidStateHandler(String),

    #[error("Invalid input provided: {0}")]
    InvalidInput(String),

    #[error("Failed to persist state: {0}")]
    PersistStateError(String),

    #[error("Failed to process state: {0}")]
    ProcessStateError(String),

    #[error("Failed to validate input: {0}")]
    ValidateInputError(String),
}
