use bonsai_trie::DBError;

use crate::Column;

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("Failed to commit DB Update: `{0}`")]
    RocksDB(#[from] rocksdb::Error),
    #[error("Failed to deserialize DB Data: `{0}`")]
    DeserializeError(#[from] parity_scale_codec::Error),
    #[error("Failed to build Uuid: `{0}`")]
    Uuid(#[from] uuid::Error),
    #[error("A value was queryied that was not initialized at column: `{0}` key: `{1}`")]
    ValueNotInitialized(Column, String),
    #[error("Format error: `{0}`")]
    Format(String),
}

#[derive(Debug, thiserror::Error)]
pub enum BonsaiDbError {
    #[error("IO error: `{0}`")]
    Io(#[from] std::io::Error),
    #[error("Failed to commit DB Update: `{0}`")]
    RocksDB(#[from] rocksdb::Error),
}

impl DBError for BonsaiDbError {}
