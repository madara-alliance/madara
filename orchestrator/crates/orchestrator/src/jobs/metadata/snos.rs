//! Metadata for SNOS (Starknet OS) jobs.

use serde::{Deserialize, Serialize};

/// Metadata specific to SNOS (Starknet OS) jobs.
///
/// # Field Management
/// - Worker-initialized fields: block_number, full_output, and path configurations
/// - Job-populated fields: snos_fact (during processing)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SnosMetadata {
    // Worker-initialized fields
    /// Block number to process
    pub block_number: u64,
    /// Whether to generate full SNOS output
    pub full_output: bool,
    /// Path to the Cairo PIE file
    pub cairo_pie_path: Option<String>,
    /// Path to the SNOS output file
    pub snos_output_path: Option<String>,
    /// Path to the program output file
    pub program_output_path: Option<String>,

    // Job-populated fields
    /// SNOS fact generated during processing
    pub snos_fact: Option<String>,
    /// SNOS total steps taken
    pub snos_n_steps: Option<usize>,
}
