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
pub const BLOB_LEN: usize = 4096; // Max number of felts (each encoded using 256 bits) that we can include in an Ethereum BLOB

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

/// Supported Starknet protocol versions by this orchestrator build.
/// This enum must be kept in sync with the SNOS library version (currently v0.13.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum StarknetVersion {
    #[serde(rename = "0.13.0")]
    V0_13_0,
    #[serde(rename = "0.13.1")]
    V0_13_1,
    #[serde(rename = "0.13.2")]
    V0_13_2,
    #[serde(rename = "0.13.3")]
    V0_13_3,
}

impl StarknetVersion {
    /// Convert version enum to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            StarknetVersion::V0_13_0 => "0.13.0",
            StarknetVersion::V0_13_1 => "0.13.1",
            StarknetVersion::V0_13_2 => "0.13.2",
            StarknetVersion::V0_13_3 => "0.13.3",
        }
    }
}

impl std::str::FromStr for StarknetVersion {
    type Err = String;

    fn from_str(version: &str) -> Result<Self, Self::Err> {
        match version {
            "0.13.0" => Ok(StarknetVersion::V0_13_0),
            "0.13.1" => Ok(StarknetVersion::V0_13_1),
            "0.13.2" => Ok(StarknetVersion::V0_13_2),
            "0.13.3" => Ok(StarknetVersion::V0_13_3),
            _ => Err(format!("Unsupported Starknet version: {}", version)),
        }
    }
}

impl std::fmt::Display for StarknetVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Supported Starknet protocol versions by this orchestrator build.
/// This list must be kept in sync with the SNOS library version (currently v0.13.3).
const SUPPORTED_STARKNET_VERSIONS: &[&str] = &["0.13.2", "0.13.3"];

/// Get the orchestrator version string for queue message filtering
pub fn get_version_string() -> String {
    format!("orchestrator-{}", ORCHESTRATOR_VERSION)
}

/// Validates if a Starknet protocol version is supported by this orchestrator.
///
/// # Arguments
/// * `version` - The Starknet version string to validate (e.g., "0.13.2")
///
/// # Returns
/// * `true` if the version is supported, `false` otherwise
/// ```
pub fn is_starknet_version_supported(version: &str) -> bool {
    SUPPORTED_STARKNET_VERSIONS.contains(&version)
}
