use std::{error, fmt};

use bonsai_trie::DBError;

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("Failed to commit DB Update: `{0}`")]
    CommitError(#[from] sp_database::error::DatabaseError),
    #[error("Failed to deserialize DB Data: `{0}`")]
    DeserializeError(#[from] scale_codec::Error),
}

// Define a custom error type
#[derive(Debug)]
pub enum BonsaiDbError {
    Io(std::io::Error),
    // Add other error types as needed
}

impl fmt::Display for BonsaiDbError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            BonsaiDbError::Io(ref err) => write!(f, "IO error: {}", err),
            // Handle other errors
        }
    }
}

impl error::Error for BonsaiDbError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            BonsaiDbError::Io(ref err) => Some(err),
            // Handle other errors
        }
    }
}

impl From<std::io::Error> for BonsaiDbError {
    fn from(err: std::io::Error) -> BonsaiDbError {
        BonsaiDbError::Io(err)
    }
}

impl DBError for BonsaiDbError {}
