use std::fmt::Display;

pub mod codec;

pub mod bonsai_identifier {
    pub const CONTRACT: &[u8] = "0xcontract".as_bytes();
    pub const CLASS: &[u8] = "0xclass".as_bytes();
    pub const TRANSACTION: &[u8] = "0xtransaction".as_bytes();
    pub const EVENT: &[u8] = "0xevent".as_bytes();
}

#[derive(thiserror::Error, Debug)]
pub enum DeoxysStorageError {
    #[error("Failed to initialize trie for {0}")]
    TrieInitError(TrieType),
    #[error("Failed to compute trie root for {0}")]
    TrieRootError(TrieType),
    #[error("Failed to merge transactional state back into {0}")]
    TrieMergeError(TrieType),
    #[error("Failed to retrieve latest id for {0}")]
    TrieIdError(TrieType),
    #[error("Failed to retrieve storage view for {0}")]
    StoraveViewError(StorageType),
    #[error("Failed to insert data into {0}")]
    StorageInsertionError(StorageType),
    #[error("Failed to retrive data from {0}")]
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
    #[error("value codec error: {0:#}")]
    Codec(#[from] codec::Error),
    #[error("bincode codec error: {0:#}")]
    Bincode(#[from] bincode::Error),
    #[error("json codec error: {0:#}")]
    Json(#[from] serde_json::Error),
    #[error("rocksdb error: {0:#}")]
    RocksDBError(#[from] rocksdb::Error),
    #[error("chain info is missing from the database")]
    MissingChainInfo,
}

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

impl Display for TrieType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let trie_type = match self {
            TrieType::Contract => "contract trie",
            TrieType::ContractStorage => "contract storage trie",
            TrieType::Class => "class trie",
        };

        write!(f, "{trie_type}")
    }
}

impl Display for StorageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let storage_type = match self {
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
        };

        write!(f, "{storage_type}")
    }
}
