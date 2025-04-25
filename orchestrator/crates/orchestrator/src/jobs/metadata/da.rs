//! Metadata for data availability (DA) jobs.

use serde::{Deserialize, Serialize};

/// Metadata specific to data availability (DA) jobs.
///
/// # Field Management
/// - Worker-initialized fields: block_number and blob_data_path
/// - Job-populated fields: tx_hash (during processing)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DaMetadata {
    // Worker-initialized fields
    /// Block number for data availability
    pub block_number: u64,
    /// Path to the blob data file
    pub blob_data_path: Option<String>,

    // Job-populated fields
    /// Transaction hash after data submission
    pub tx_hash: Option<String>,
}
