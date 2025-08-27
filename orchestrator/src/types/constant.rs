use lazy_static::lazy_static;
use num_bigint::{BigUint, ToBigUint};
use std::str::FromStr;

pub const BLOB_DATA_FILE_NAME: &str = "blob_data.txt";
pub const SNOS_OUTPUT_FILE_NAME: &str = "snos_output.json";
pub const PROGRAM_OUTPUT_FILE_NAME: &str = "program_output.txt";
pub const CAIRO_PIE_FILE_NAME: &str = "cairo_pie.zip";
pub const PROOF_FILE_NAME: &str = "proof.json";
pub const PROOF_PART2_FILE_NAME: &str = "proof_part2.json";
pub const ON_CHAIN_DATA_FILE_NAME: &str = "onchain_data.json";

pub const STORAGE_STATE_UPDATE_DIR: &str = "state_update";
pub const STORAGE_BLOB_DIR: &str = "blob";
pub const STORAGE_ARTIFACTS_DIR: &str = "artifacts";
pub const BLOB_LEN: usize = 4096;
pub const MAX_BLOBS: usize = 6;
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
