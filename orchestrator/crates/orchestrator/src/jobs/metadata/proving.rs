//! Metadata for proving jobs.

use serde::{Deserialize, Serialize};

/// Input type specification for proving jobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProvingInputTypePath {
    /// Path to an existing proof
    Proof(String),
    /// Path to a Cairo PIE file
    CairoPie(String),
}

/// Metadata specific to proving jobs.
///
/// # Field Management
/// All fields are initialized by the worker during job creation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ProvingMetadata {
    /// Block number to prove
    pub block_number: u64,
    /// Path to the input file (proof or Cairo PIE)
    pub input_path: Option<ProvingInputTypePath>,
    /// SNOS fact to check for on-chain registration. If `None`, no on-chain check is performed. If
    /// `Some(value)`, it checks for `value` on the chain.
    pub ensure_on_chain_registration: Option<String>,
    /// Path where the generated proof should be downloaded. If `None`, the proof will not be
    /// downloaded. If `Some(value)`, the proof will be downloaded and stored to the specified path
    /// in the provided storage.
    pub download_proof: Option<String>,
    pub n_steps: Option<usize>,
}
