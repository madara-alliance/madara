//! Error types for Pathfinder RPC operations and proof verification.

use starknet_types_core::felt::Felt;

use crate::types::{Height, TrieNode};

/// Errors that can occur during Pathfinder RPC client operations.
///
/// This enum represents various error conditions that can arise when interacting
/// with Pathfinder RPC endpoints, including network errors, parsing errors, and
/// custom application errors.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A network or HTTP request error occurred.
    #[error("Encountered a request error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    /// A custom application-specific error occurred.
    #[error("Encountered a custom error: {0}")]
    CustomError(String),

    /// Failed to convert a response to a ContractProof.
    #[error("Failed to convert RPC response to SNOS Proof: {0}")]
    ProofConversionError(String),

    /// Invalid URL or endpoint configuration.
    #[error("Invalid URL or endpoint configuration: {0}")]
    ConfigurationError(String),

    /// Request timeout occurred.
    #[error("Request timeout after {0} seconds")]
    TimeoutError(u64),

    /// Invalid response format from the server.
    #[error("Invalid response format: {0}")]
    InvalidResponseFormat(String),
}

/// Errors that can occur during proof verification operations.
///
/// This enum represents various error conditions that can arise when verifying
/// storage proofs, including non-existence proofs, invalid node hashes, and
/// conversion errors.
#[derive(Debug, thiserror::Error)]
pub enum ProofVerificationError {
    /// A non-inclusion proof was encountered for a specific key.
    ///
    /// This error occurs when the proof indicates that a key does not exist in
    /// the tree at the specified height. The error includes the key, height, and
    /// the trie node that was encountered.
    #[error("Non-inclusion proof for key {}. Height {}.", key.to_hex_string(), height.0)]
    NonExistenceProof { key: Felt, height: Height, node: TrieNode },

    /// Proof verification failed due to invalid node hash.
    ///
    /// This error occurs when the hash of a child node does not match the hash
    /// specified in the parent node, indicating a corrupted or invalid proof.
    #[error("Proof verification failed, node_hash {node_hash:x} != parent_hash {parent_hash:x}")]
    InvalidChildNodeHash { node_hash: Felt, parent_hash: Felt },

    /// The proof is empty or contains no nodes.
    #[error("Proof is empty")]
    EmptyProof,

    /// A conversion error occurred during proof processing.
    #[error("Conversion error: {0}")]
    ConversionError(String),

    /// A general proof verification error occurred.
    #[error("Proof error: {0}")]
    ProofError(String),

    /// Invalid proof structure or format.
    #[error("Invalid proof structure: {0}")]
    InvalidProofStructure(String),

    /// Tree height exceeds the maximum allowed height.
    #[error("Tree height {height} exceeds maximum allowed height")]
    InvalidTreeHeight { height: u64 },
}
