use crate::codec;

#[derive(thiserror::Error, Debug)]
pub enum DeoxysStorageError {
    #[error("Format error: `{0}`")]
    Format(String),
    #[error("Failed to initialize trie for {0}")]
    TrieInitError(TrieType),
    #[error("Failed to compute trie root for {0}")]
    TrieRootError(TrieType),
    #[error("Failed to merge transactional state back into {0}")]
    TrieMergeError(TrieType),
    #[error("Failed to retrieve latest id for {0}")]
    TrieIdError(TrieType),
    #[error("Failed to retrieve storage view for {0}")]
    StorageViewError(StorageType),
    #[error("Failed to insert data into {0}")]
    StorageInsertionError(StorageType),
    #[error("Failed to retrieve data from {0}")]
    StorageRetrievalError(StorageType),
    #[error("Failed to commit to {0}")]
    StorageCommitError(StorageType),
    #[error("Failed to encode {0}")]
    StorageEncodeError(StorageType),
    #[error("Failed to decode {0}")]
    StorageDecodeError(StorageType),
    #[error("Failed to revert {0} to block {1}")]
    StorageRevertError(StorageType, u64),
    #[error("Invalid block number")]
    InvalidBlockNumber,
    #[error("Invalid nonce")]
    InvalidNonce,
    #[error("Failed to compile class: {0}")]
    CompilationClassError(String),
    #[error("rocksdb error: {0:#}")]
    RocksDB(#[from] rocksdb::Error),
    #[error("value codec error: {0:#}")]
    Codec(#[from] codec::Error),
    #[error("bincode codec error: {0:#}")]
    Bincode(#[from] bincode::Error),
    #[error("json codec error: {0:#}")]
    Json(#[from] serde_json::Error),
    #[error("chain info is missing from the database")]
    MissingChainInfo,
}

// #[derive(thiserror::Error, Debug)]
// pub enum DbError {
//     #[error("Failed to commit DB Update: `{0}`")]
//     RocksDB(#[from] rocksdb::Error),
//     #[error("Format error: `{0}`")]
//     Format(String),
//     #[error("value codec error: {0}")]
//     Bincode(#[from] bincode::Error),
// }

impl bonsai_trie::DBError for DeoxysStorageError {}

#[derive(Debug)]
pub enum TrieType {
    Contract,
    ContractStorage,
    Class,
}

#[derive(Debug)]
pub enum StorageType {
    Contract,
    ContractStorage,
    ContractClassData,
    CompiledContractClass,
    ContractData,
    ContractAbi,
    ContractClassHashes,
    Class,
    BlockNumber,
    BlockHash,
    BlockStateDiff,
}

impl TrieType {
    fn as_str(&self) -> &str {
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

impl StorageType {
    fn as_str(&self) -> &str {
        match self {
            StorageType::Contract => "contract",
            StorageType::ContractStorage => "contract storage",
            StorageType::Class => "class storage",
            StorageType::ContractClassData => "class definition storage",
            StorageType::CompiledContractClass => "compiled class storage",
            StorageType::ContractAbi => "class abi storage",
            StorageType::BlockNumber => "block number storage",
            StorageType::BlockHash => "block hash storage",
            StorageType::BlockStateDiff => "block state diff storage",
            StorageType::ContractClassHashes => "contract class hashes storage",
            StorageType::ContractData => "contract class data storage",
        }
    }
}

impl std::fmt::Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
