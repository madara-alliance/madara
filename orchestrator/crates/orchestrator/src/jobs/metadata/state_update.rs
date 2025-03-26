//! Metadata for state update jobs.

use serde::{Deserialize, Serialize};

/// Metadata specific to state update jobs.
///
/// # Field Management
/// - Worker-initialized fields: blocks and paths configurations
/// - Job-populated fields: last_failed_block_no and tx_hashes (during processing)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StateUpdateMetadata {
    // Worker-initialized fields
    /// Block numbers that need to be settled
    pub blocks_to_settle: Vec<u64>,
    /// Paths to SNOS output files for each block
    pub snos_output_paths: Vec<String>,
    /// Paths to program output files for each block
    pub program_output_paths: Vec<String>,
    /// Paths to blob data files for each block
    pub blob_data_paths: Vec<String>,

    // Job-populated fields
    /// Last block number that failed processing
    pub last_failed_block_no: Option<u64>,
    /// Transaction hashes for processed blocks
    pub tx_hashes: Vec<String>,
}
