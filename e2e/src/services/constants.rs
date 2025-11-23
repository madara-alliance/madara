// =============================================================================
// CONSTANTS
// =============================================================================

use std::path::PathBuf;
use std::sync::LazyLock;
use tokio::time::Duration;

// =============================================================================
// GENERAL / SHARED
// =============================================================================
pub static REPO_ROOT: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().expect("Failed to get workspace root").to_path_buf()
});
pub const DEFAULT_SERVICE_HOST: &str = "127.0.0.1";
pub const BINARY_DIR: &str = "target/release";
pub const DATA_DIR: &str = "e2e_data";
pub const BLOCK_NOT_FOUND_ERROR_CODE: u64 = 24;

// =============================================================================
// SERVER
// =============================================================================
pub const BUFFER_CAPACITY: usize = 65536;
pub const CONNECTION_ATTEMPTS: usize = 30;
pub const CONNECTION_DELAY_MS: u64 = 1000;

// =============================================================================
// ANVIL SERVICE
// =============================================================================
pub const ANVIL_DATABASE_FILE: &str = "anvil.json";
pub const ANVIL_BLOCK_TIME: f64 = 1_f64;
pub const ANVIL_PORT: u16 = 8545;
pub const ANVIL_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub const ANVIL_CHAIN_ID: u64 = 31337;

// =============================================================================
// BOOTSTRAPPER SERVICE
// =============================================================================
pub const BOOTSTRAPPER_BINARY: &str = "bootstrapper";
pub static BOOTSTRAPPER_SETUP_L1_TIMEOUT: LazyLock<Duration> = LazyLock::new(|| Duration::from_secs(120));
pub static BOOTSTRAPPER_SETUP_L2_TIMEOUT: LazyLock<Duration> = LazyLock::new(|| Duration::from_secs(1200));
pub const BOOTSTRAPPER_CONFIG: &str = "e2e/config/bootstrapper.json";
pub const BOOTSTRAPPER_ADDRESSES_FILE: &str = "addresses.json";

// =============================================================================
// LOCALSTACK SERVICE
// =============================================================================
pub const LOCALSTACK_IMAGE: &str =
    "localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314";
pub const LOCALSTACK_PORT: u16 = 4566;
pub const LOCALSTACK_CONTAINER: &str = "localstack-service";

// =============================================================================
// MONGODB SERVICE
// =============================================================================
pub const MONGODB_IMAGE: &str = "mongo:latest";
pub const MONGODB_PORT: u16 = 27017;
pub const MONGODB_CONTAINER: &str = "mongodb-service";

// =============================================================================
// MADARA SERVICE
// =============================================================================
pub const MADARA_NAME: &str = "madara";
pub const MADARA_DATABASE_DIR: &str = "madara-db";
pub const MADARA_BINARY: &str = "madara";
pub const MADARA_CONFIG: &str = "e2e/config/madara.yaml";
pub const MADARA_RPC_PORT: u16 = 9944;
pub const MADARA_RPC_ADMIN_PORT: u16 = 9943;
pub const MADARA_GATEWAY_PORT: u16 = 8080;
pub static MADARA_WAITING_DURATION: LazyLock<Duration> = LazyLock::new(|| Duration::from_secs(1));

// =============================================================================
// MOCK PROVER SERVICE
// =============================================================================
pub const MOCK_PROVER_BINARY: &str = "mock-atlantic-server";
pub const MOCK_PROVER_PORT: u16 = 3001;

// =============================================================================
// PATHFINDER SERVICE
// =============================================================================
pub const PATHFINDER_PORT: u16 = 9545;
pub const PATHFINDER_BINARY: &str = "pathfinder";
pub const PATHFINDER_DATABASE_DIR: &str = "pathfinder-db";

// =============================================================================
// ORCHESTRATOR SERVICE
// =============================================================================
pub const ORCHESTRATOR_BINARY: &str = "orchestrator";
pub const ORCHESTRATOR_DATABASE_NAME: &str = "orchestrator";

// =============================================================================
// MOCK VERIFIER DEPLOYER SERVICE
// =============================================================================
pub const MOCK_VERIFIER_TIMEOUT_SECS: u64 = 300;
pub const MOCK_VERIFIER_DEPLOYER_SCRIPT: &str = "test_utils/scripts/deploy_dummy_verifier.sh";
pub const MOCK_VERIFIER_CONTRACT: &str = "test_utils/scripts/artifacts/MockGPSVerifier.sol:MockGPSVerifier";
pub const MOCK_VERIFIER_ADDRESS_FILE: &str = "verifier_address.txt";
