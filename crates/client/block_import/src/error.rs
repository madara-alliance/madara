use std::borrow::Cow;

use dp_block::{BlockCommitmentError, BlockValidationError};
use starknet_core::types::Felt;

use dc_db::DeoxysStorageError;

#[derive(Debug, thiserror::Error)]
pub enum BlockImportError {
    #[error("Block commitment error: {0}")]
    CommitmentError(#[from] BlockCommitmentError),

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

    #[error("Invalid block: {0}")]
    BlockValidationError(#[from] BlockValidationError),
}
