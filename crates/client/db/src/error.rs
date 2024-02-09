use std::{error, fmt};

use bonsai_trie::DBError;

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("Failed to commit DB Update: `{0}`")]
    CommitError(#[from] sp_database::error::DatabaseError),
    #[error("Failed to deserialize DB Data: `{0}`")]
    DeserializeError(#[from] parity_scale_codec::Error),
    #[error("Failed to build Uuid: `{0}`")]
    Uuid(#[from] uuid::Error),
    #[error("A value was queryied that was not initialized at column: `{0}` key: `{1}`")]
    ValueNotInitialized(u32, String),
}

#[derive(Debug)]
pub enum BonsaiDbError {
    Io(std::io::Error),
}

impl fmt::Display for BonsaiDbError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            BonsaiDbError::Io(ref err) => write!(f, "IO error: {}", err),
        }
    }
}

impl error::Error for BonsaiDbError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            BonsaiDbError::Io(ref err) => Some(err),
        }
    }
}

impl From<std::io::Error> for BonsaiDbError {
    fn from(err: std::io::Error) -> BonsaiDbError {
        BonsaiDbError::Io(err)
    }
}

impl DBError for BonsaiDbError {}
