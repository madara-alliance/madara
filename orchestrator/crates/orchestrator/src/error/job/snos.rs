use crate::error::job::fact::FactError;
use crate::error::other::OtherError;
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum SnosError {
    #[error("Block numbers to run must be specified (snos job #{internal_id:?})")]
    UnspecifiedBlockNumber { internal_id: String },
    #[error("No block numbers found (snos job #{internal_id:?})")]
    BlockNumberNotFound { internal_id: String },
    #[error("Invalid specified block number \"{block_number:?}\" (snos job #{internal_id:?})")]
    InvalidBlockNumber { internal_id: String, block_number: String },

    #[error("Could not serialize the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnserializable { internal_id: String, message: String },
    #[error("Could not store the Cairo Pie (snos job #{internal_id:?}): {message}")]
    CairoPieUnstorable { internal_id: String, message: String },

    #[error("Could not serialize the Snos Output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnserializable { internal_id: String, message: String },
    #[error("Could not serialize the Program Output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnserializable { internal_id: String, message: String },
    #[error("Could not store the Snos output (snos job #{internal_id:?}): {message}")]
    SnosOutputUnstorable { internal_id: String, message: String },
    #[error("Could not store the Program output (snos job #{internal_id:?}): {message}")]
    ProgramOutputUnstorable { internal_id: String, message: String },

    // ProveBlockError from Snos is not usable with #[from] since it does not implement PartialEq.
    // Instead, we convert it to string & pass it into the [SnosExecutionError] error.
    #[error("Error while running SNOS (snos job #{internal_id:?}): {message}")]
    SnosExecutionError { internal_id: String, message: String },

    #[error("Error when calculating fact info: {0}")]
    FactCalculationError(#[from] FactError),

    #[error("Other error: {0}")]
    Other(#[from] OtherError),
}
