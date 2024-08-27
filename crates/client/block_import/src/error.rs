use dc_db::DeoxysStorageError;
use starknet_core::types::Felt;
use std::borrow::Cow;

#[derive(Debug, thiserror::Error)]
pub enum BlockImportError {
    #[error("Transaction count and receipt count do not match: {receipts} receipts != {transactions} transactions")]
    TransactionEqualReceiptCount { receipts: usize, transactions: usize },

    #[error("Transaction hash mismatch for index #{index}: expected {expected:#x}, got {got:#x}")]
    TransactionHash { index: usize, got: Felt, expected: Felt },
    #[error("Transaction count mismatch: expected {expected}, got {got}")]
    TransactionCount { got: u64, expected: u64 },
    #[error("Transaction commitment mismatch: expected {expected:#x}, got {got:#x}")]
    TransactionCommitment { got: Felt, expected: Felt },

    #[error("Event count mismatch: expected {expected}, got {got}")]
    EventCount { got: u64, expected: u64 },
    #[error("Event commitment mismatch: expected {expected:#x}, got {got:#x}")]
    EventCommitment { got: Felt, expected: Felt },

    #[error("State diff length mismatch: expected {expected}, got {got}")]
    StateDiffLength { got: u64, expected: u64 },
    #[error("State diff commitment mismatch: expected {expected:#x}, got {got:#x}")]
    StateDiffCommitment { got: Felt, expected: Felt },

    #[error("Receipt commitment mismatch: expected {expected:#x}, got {got:#x}")]
    ReceiptCommitment { got: Felt, expected: Felt },

    #[error("Class hash mismatch: expected {expected:#x}, got {got:#x}")]
    ClassHash { got: Felt, expected: Felt },
    #[error("Compiled class hash mismatch for class hash {class_hash:#x}: expected {expected:#x}, got {got:#x}")]
    CompiledClassHash { class_hash: Felt, got: Felt, expected: Felt },
    #[error("Class with hash {class_hash:#x} failed to compile: {error}")]
    CompilationClassError { class_hash: Felt, error: String },

    #[error("Block hash mismatch: expected {expected:#x}, got {got:#x}")]
    BlockHash { got: Felt, expected: Felt },

    #[error("Block order mismatch: database expects to import block #{expected}, trying to import #{got}")]
    LatestBlockN { expected: u64, got: u64 },
    #[error("Parent hash mismatch: expected {expected:#x}, got {got:#x}")]
    ParentHash { got: Felt, expected: Felt },
    #[error("Global state root mismatch: expected {expected:#x}, got {got:#x}")]
    GlobalStateRoot { got: Felt, expected: Felt },

    #[error("Internal database error while {context}: {error:#}")]
    InternalDb { context: Cow<'static, str>, error: DeoxysStorageError },
    #[error("Internal error: {0}")]
    Internal(Cow<'static, str>),
}
