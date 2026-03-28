//! Constants used throughout the RPC client

pub const STARKNET_RPC_VERSION: &str = "v0_10";

pub const SNOS_RPC_URL_ENV: &str = "SNOS_RPC_URL";

/// Default height of the storage tree in Pathfinder.
///
/// This constant represents the maximum height of the Patricia Merkle tree used
/// for storage in Pathfinder. It's used in proof verification and tree traversal.
pub const DEFAULT_STORAGE_TREE_HEIGHT: u64 = 251;

/// Maximum number of storage keys that can be requested in a single proof request.
///
/// This constant defines the limit on the number of storage keys that can be
/// included in a single `starknet_getStorageProof` request to Pathfinder.
pub const MAX_STORAGE_KEYS_PER_REQUEST: usize = 90;

/// Maximum number of concurrent proof requests to send at a time.
///
/// This constant defines the limit on the number of concurrent RPC requests
/// that can be made when fetching storage proofs for multiple key chunks.
/// Higher values increase throughput but may overwhelm the RPC server.
pub const MAX_CONCURRENT_PROOF_REQUESTS: usize = 10;

/// Default timeout for RPC requests in seconds.
///
/// This constant defines the default timeout duration for HTTP requests to
/// Pathfinder RPC endpoints.
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: u64 = 30;
