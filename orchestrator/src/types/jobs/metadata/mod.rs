use crate::types::error::TypeError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Common metadata fields shared across all job types.
///
/// # Field Management
/// These fields are automatically managed by the job processing system and should not
/// be modified directly by workers or jobs. The system uses these fields to:
/// - Track processing and verification attempts
/// - Record completion timestamps
/// - Store failure information
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CommonMetadata {
    /// Number of times the job has been processed
    pub process_attempt_no: u64,
    /// Number of times the job has been retried after processing failures
    pub process_retry_attempt_no: u64,
    /// Number of times the job has been verified
    pub verification_attempt_no: u64,
    /// Number of times the job has been retried after verification failures
    pub verification_retry_attempt_no: u64,
    /// Timestamp when job processing started
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub process_started_at: Option<DateTime<Utc>>,
    /// Timestamp when job processing completed
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub process_completed_at: Option<DateTime<Utc>>,
    /// Timestamp when job verification started
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub verification_started_at: Option<DateTime<Utc>>,
    /// Timestamp when job verification completed
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub verification_completed_at: Option<DateTime<Utc>>,
    /// Reason for job failure if any
    pub failure_reason: Option<String>,
}

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

/// Input type specification for proving jobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProvingInputType {
    /// Path to an existing proof
    Proof(String),
    /// Path to a Cairo PIE file
    CairoPie(String),
}

/// Metadata specific to aggregator job
///
/// # Field Management
/// TODO: update this
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AggregatorMetadata {
    // Worker populated field
    /// Batch number corresponding to the Aggregator job
    pub batch_num: u64,
    /// Bucker ID received from the prover client
    pub bucket_id: String,
    /// SNOS fact to check for on-chain registration during verification.
    /// If `None`, no on-chain check is performed.
    /// If `Some(value)`, it checks for `value` on the chain.
    pub ensure_on_chain_registration: Option<String>,
    /// Path where the generated proof should be downloaded. If `None`, the proof will not be
    /// downloaded. If `Some(value)`, the proof will be downloaded and stored to the specified path
    /// in the provided storage.
    pub download_proof: Option<String>,
    /// Path of blob data
    pub blob_data_path: String,

    // Job populated field
    // We'll get these from the Prover client after the aggregator job is completed
    /// Path to the Cairo PIE file
    pub cairo_pie_path: String,
    /// Path to the SNOS output file
    pub snos_output_path: String,
    /// Path to the program output file
    pub program_output_path: String,
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
    pub input_path: Option<ProvingInputType>,
    /// SNOS fact to check for on-chain registration. If `None`, no on-chain check is performed. If
    /// `Some(value)`, it checks for `value` on the chain.
    pub ensure_on_chain_registration: Option<String>,
    /// Path where the generated proof should be downloaded. If `None`, the proof will not be
    /// downloaded. If `Some(value)`, the proof will be downloaded and stored to the specified path
    /// in the provided storage.
    pub download_proof: Option<String>,
    pub n_steps: Option<usize>,
    /// Bucket ID received from the prover client.
    /// If None, it's assumed that the bucket ID is not needed (i.e., not using Applicative Recursion)
    pub bucked_id: Option<String>,
    /// Index of the block within the bucket.
    /// If None, it's assumed that we are not using Applicative Recursion
    pub bucket_job_index: Option<u64>,
}

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

/// Metadata specific to state update jobs.
///
/// # Field Management
/// - Worker-initialized fields: blocks and paths configurations
/// - Job-populated fields: last_failed_block_no and tx_hashes (during processing)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StateUpdateMetadata {
    // Worker-initialized fields
    /// Block numbers that need to be settled
    // pub blocks_to_settle: Vec<u64>,
    /// Batch numbers that need to be settled
    pub batches_to_settle: Vec<u64>,
    /// Paths to SNOS output files for each block/batch
    pub snos_output_paths: Vec<String>,
    /// Paths to program output files for each block/batch
    pub program_output_paths: Vec<String>,
    /// Paths to blob data files for each block/batch
    pub blob_data_paths: Vec<String>,

    // Job-populated fields
    /// Last block number that failed processing
    // pub last_failed_block_no: Option<u64>,
    /// Last batch number that failed processing
    pub last_failed_batch_no: Option<u64>,
    /// Transaction hashes for processed blocks/batches
    pub tx_hashes: Vec<String>,
}

/// Enum containing all possible job-specific metadata types.
///
/// This enum is used to provide type-safe access to job-specific metadata
/// while maintaining a common interface for job processing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum JobSpecificMetadata {
    /// SNOS job metadata
    Snos(SnosMetadata),
    /// State update job metadata
    StateUpdate(StateUpdateMetadata),
    /// Proving job metadata
    Proving(ProvingMetadata),
    /// Data availability job metadata
    Da(DaMetadata),
    /// Aggregator job metadata
    Aggregator(AggregatorMetadata),
}

/// Macro to implement TryInto for JobSpecificMetadata variants
macro_rules! impl_try_into_metadata {
    ($variant:ident, $type:ident) => {
        impl TryInto<$type> for JobSpecificMetadata {
            type Error = TypeError;

            fn try_into(self) -> Result<$type, Self::Error> {
                match self {
                    JobSpecificMetadata::$variant(metadata) => Ok(metadata),
                    _ => Err(TypeError::InvalidMetadataVariantMap(format!(
                        "Invalid metadata type: expected {} metadata",
                        stringify!($variant)
                    ))),
                }
            }
        }
    };
}

// Implement TryInto for all metadata types
impl_try_into_metadata!(Snos, SnosMetadata);
impl_try_into_metadata!(Proving, ProvingMetadata);
impl_try_into_metadata!(Da, DaMetadata);
impl_try_into_metadata!(StateUpdate, StateUpdateMetadata);
impl_try_into_metadata!(Aggregator, AggregatorMetadata);

/// Complete job metadata containing both common and job-specific fields.
///
/// # Field Management
/// - `common`: Managed automatically by the job processing system
/// - `specific`: Contains job-type specific fields managed by workers and jobs
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JobMetadata {
    /// Common metadata fields shared across all job types
    pub common: CommonMetadata,
    /// Job-specific metadata fields
    pub specific: JobSpecificMetadata,
}
