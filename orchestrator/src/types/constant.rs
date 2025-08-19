pub const BLOB_DATA_FILE_NAME: &str = "blob_data.txt";
pub const SNOS_OUTPUT_FILE_NAME: &str = "snos_output.json";
pub const PROGRAM_OUTPUT_FILE_NAME: &str = "program_output.txt";
pub const CAIRO_PIE_FILE_NAME: &str = "cairo_pie.zip";
pub const STORAGE_STATE_UPDATE_DIR: &str = "state_update";
// TODO: Remove this constant when `assign_batch_to_block` method is updated
pub const MAX_BATCH_SIZE: u64 = 50;
pub const ON_CHAIN_DATA_FILE_NAME: &str = "onchain_data.json";
pub const PROOF_FILE_NAME: &str = "proof.json";
pub const PROOF_PART2_FILE_NAME: &str = "proof_part2.json";
pub const BOOT_LOADER_PROGRAM_CONTRACT: &str = "0x5ab580b04e3532b6b18f81cfa654a05e29dd8e2352d88df1e765a84072db07";

/// Chunk size for reading files in bytes, when streaming data from the file
pub const BYTE_CHUNK_SIZE: usize = 8192; // 8KB chunks

/// Version of the Madara client
pub const MADARA_VERSION: &str = "0.1.0";

/// Version of the Starknet client
pub const STARKNET_VERSION: &str = "0.1.0";

/// Version of the Orchestrator
pub const ORCHESTRATOR_VERSION: &str = "1.0.0";

/// Generate a version string combining Starknet and Orchestrator versions
/// This format allows for future extensibility with additional dependencies
pub fn generate_version_string() -> String {
    format!("starknet-{}::orchestrator-{}", STARKNET_VERSION, ORCHESTRATOR_VERSION)
}
