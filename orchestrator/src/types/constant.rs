use lazy_static::lazy_static;
use num_bigint::{BigUint, ToBigUint};
use std::str::FromStr;

// Storage constants and path helpers live in types::storage_layout.
// Re-exported here for backward compatibility across the codebase.
pub use crate::types::storage_layout::{
    get_batch_artifact_file, get_batch_artifacts_dir, get_batch_blob_dir, get_batch_blob_file,
    get_batch_state_update_file, get_snos_batch_dir, BLOB_DATA_FILE_NAME, CAIRO_PIE_FILE_NAME, DA_SEGMENT_FILE_NAME,
    ON_CHAIN_DATA_FILE_NAME, PROGRAM_OUTPUT_FILE_NAME, PROOF_FILE_NAME, PROOF_PART2_FILE_NAME, SNOS_OUTPUT_FILE_NAME,
    STORAGE_ARTIFACTS_DIR, STORAGE_BATCH_SUBDIR, STORAGE_BLOB_DIR, STORAGE_CLEANUP_LOCK_DURATION,
    STORAGE_CLEANUP_MAX_JOBS_PER_RUN, STORAGE_CLEANUP_WORKER_KEY, STORAGE_EXPIRATION_TAG_KEY,
    STORAGE_EXPIRATION_TAG_VALUE, STORAGE_LIFECYCLE_RULE_ID, STORAGE_STATE_UPDATE_DIR,
};
pub const BLOB_LEN: usize = 4096;
pub const MAX_BLOBS: usize = 6; // TODO: This should be configurable via ENV or config file
pub const MAX_BLOB_SIZE: usize = BLOB_LEN * MAX_BLOBS; // This represents the maximum size of data that you can use in a single transaction

pub const BOOT_LOADER_PROGRAM_CONTRACT: &str = "0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07";

/// Chunk size for reading files in bytes when streaming data from the file
pub const BYTE_CHUNK_SIZE: usize = 8192; // 8KB chunks

lazy_static! {
    /// EIP-4844 BLS12-381 modulus.
    ///
    /// As defined in https://eips.ethereum.org/EIPS/eip-4844

    /// Generator of the group of evaluation points (EIP-4844 parameter).
    pub static ref GENERATOR: BigUint = BigUint::from_str(
        "39033254847818212395286706435128746857159659164139250548781411570340225835782",
    )
    .unwrap();

    pub static ref BLS_MODULUS: BigUint = BigUint::from_str(
        "52435875175126190479447740508185965837690552500527637822603658699938581184513",
    )
    .unwrap();
    pub static ref TWO: BigUint = 2u32.to_biguint().unwrap();
    pub static ref ONE: BigUint = 1u32.to_biguint().unwrap();
}

/// Version of the Orchestrator - loaded from Cargo.toml at compile time
pub const ORCHESTRATOR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Message attribute name for orchestrator version in queue messages
pub const ORCHESTRATOR_VERSION_ATTRIBUTE: &str = "OrchestratorVersion";

/// Get the orchestrator version string for queue message filtering
pub fn get_version_string() -> String {
    format!("orchestrator-{}", ORCHESTRATOR_VERSION)
}
