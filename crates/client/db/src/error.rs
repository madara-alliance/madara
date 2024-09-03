use crate::Column;
use std::borrow::Cow;

#[derive(thiserror::Error, Debug)]
pub enum MadaraStorageError {
    #[error("Bonsai error: {0}")]
    BonsaiStorageError(bonsai_trie::BonsaiStorageError<DbError>),
    #[error("Rocksdb error: {0:#}")]
    RocksDB(#[from] rocksdb::Error),
    #[error("Bincode error: {0}")]
    Bincode(#[from] bincode::Error),
    #[error("Failed to compile class: {0}")]
    CompilationClassError(String),
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Invalid nonce")]
    InvalidNonce,
    #[error("Chain info is missing from the database")]
    MissingChainInfo,
    #[error("Inconsistent storage")]
    InconsistentStorage(Cow<'static, str>),
    #[error("Cannot create a pending block of the genesis block of a chain")]
    PendingCreationNoGenesis,
}

impl From<bonsai_trie::BonsaiStorageError<DbError>> for MadaraStorageError {
    fn from(e: bonsai_trie::BonsaiStorageError<DbError>) -> Self {
        MadaraStorageError::BonsaiStorageError(e)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum DbError {
    #[error("Failed to commit DB Update: `{0}`")]
    RocksDB(#[from] rocksdb::Error),
    #[error("A value was queried that was not initialized at column: `{0}` key: `{1}`")]
    ValueNotInitialized(Column, String),
    #[error("Format error: `{0}`")]
    Format(String),
    #[error("Value codec error: {0}")]
    Bincode(#[from] bincode::Error),
}

impl bonsai_trie::DBError for DbError {}

#[derive(Debug)]
pub enum TrieType {
    Contract,
    ContractStorage,
    Class,
}

impl TrieType {
    fn as_str(&self) -> &'static str {
        match self {
            TrieType::Contract => "contract",
            TrieType::ContractStorage => "contract storage",
            TrieType::Class => "class",
        }
    }
}

impl std::fmt::Display for TrieType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
