//! Storage key layout and constants for orchestrator artifacts in object storage (e.g., S3).
//!
//! This module is the single source of truth for storage paths, file names, and
//! lifecycle tagging keys used by the orchestrator.
//!
//! # Layout overview
//! - Aggregator batch artifacts:
//!   - `artifacts/batch/<batch_index>/...`
//!   - `blob/batch/<batch_index>/<blob_index>.txt`
//!   - `state_update/batch/<batch_index>.json`
//! - Root-level batch artifacts (SNOS + L3):
//!   - `<batch_or_block>/cairo_pie.zip`
//!   - `<batch_or_block>/snos_output.json`
//!   - `<batch_or_block>/program_output.txt`
//!   - `<batch_or_block>/onchain_data.json` (configured, not currently written)
//!   - `<block_no>/blob_data.txt` (L3 DA)
//!   - `<block_no>/proof.json` (L3 proof download)
//!   - `<block_no>/proof_part2.json` (L3 proof registration download)

/// Blob data file name (DA payload).
pub const BLOB_DATA_FILE_NAME: &str = "blob_data.txt";
/// SNOS output file name.
pub const SNOS_OUTPUT_FILE_NAME: &str = "snos_output.json";
/// Program output file name.
pub const PROGRAM_OUTPUT_FILE_NAME: &str = "program_output.txt";
/// Cairo PIE file name.
pub const CAIRO_PIE_FILE_NAME: &str = "cairo_pie.zip";
/// Prover proof file name.
pub const PROOF_FILE_NAME: &str = "proof.json";
/// Prover proof (part2) file name.
pub const PROOF_PART2_FILE_NAME: &str = "proof_part2.json";
/// On-chain data file name.
pub const ON_CHAIN_DATA_FILE_NAME: &str = "onchain_data.json";
/// DA segment file name.
pub const DA_SEGMENT_FILE_NAME: &str = "da_blob.json";

/// Root directory for state update artifacts.
pub const STORAGE_STATE_UPDATE_DIR: &str = "state_update";
/// Root directory for blob artifacts.
pub const STORAGE_BLOB_DIR: &str = "blob";
/// Root directory for general artifacts.
pub const STORAGE_ARTIFACTS_DIR: &str = "artifacts";
/// Subdirectory used for batch-scoped artifacts.
pub const STORAGE_BATCH_SUBDIR: &str = "batch";

/// Returns the artifacts directory for a given batch index.
pub fn get_batch_artifacts_dir(batch_index: u64) -> String {
    format!("{}/{}/{}", STORAGE_ARTIFACTS_DIR, STORAGE_BATCH_SUBDIR, batch_index)
}

/// Returns the full key for a batch-scoped artifact.
pub fn get_batch_artifact_file(batch_index: u64, filename: &str) -> String {
    format!("{}/{}", get_batch_artifacts_dir(batch_index), filename)
}

/// Returns the blob directory for a given batch index.
pub fn get_batch_blob_dir(batch_index: u64) -> String {
    format!("{}/{}/{}", STORAGE_BLOB_DIR, STORAGE_BATCH_SUBDIR, batch_index)
}

/// Returns the full key for a batch-scoped blob file.
pub fn get_batch_blob_file(batch_index: u64, blob_index: u64) -> String {
    format!("{}/{}.txt", get_batch_blob_dir(batch_index), blob_index)
}

/// Returns the full key for a batch-scoped state update file.
pub fn get_batch_state_update_file(batch_index: u64) -> String {
    format!("{}/{}/{}.json", STORAGE_STATE_UPDATE_DIR, STORAGE_BATCH_SUBDIR, batch_index)
}

/// SNOS batch directory at root level.
pub fn get_snos_batch_dir(snos_batch_index: u64) -> String {
    format!("{}", snos_batch_index)
}

/// Storage tag key used for lifecycle expiration.
pub const STORAGE_EXPIRATION_TAG_KEY: &str = "expire-after-settlement";
/// Storage tag value used for lifecycle expiration.
pub const STORAGE_EXPIRATION_TAG_VALUE: &str = "true";
/// Lifecycle rule ID for expiration.
pub const STORAGE_LIFECYCLE_RULE_ID: &str = "expire-settled-artifacts";

/// Storage cleanup worker lock key.
pub const STORAGE_CLEANUP_WORKER_KEY: &str = "StorageCleanupWorker";
/// Maximum number of jobs to process per cleanup run.
pub const STORAGE_CLEANUP_MAX_JOBS_PER_RUN: usize = 200;
/// Storage cleanup lock duration (seconds).
pub const STORAGE_CLEANUP_LOCK_DURATION: u64 = 300; // 5 minutes
