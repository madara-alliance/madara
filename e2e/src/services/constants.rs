pub const ANVIL_DEFAULT_DATABASE_NAME: &str = "anvil.json";
pub const DEFAULT_BOOTSTRAPPER_BINARY: &str = "../target/release/bootstrapper";
pub const DEFAULT_BOOTSTRAPPER_CONFIG: &str = "./config/bootstrapper.json";
pub const BOOTSTRAPPER_DEFAULT_ADDRESS_PATH: &str = "addresses.json";
pub const DEFAULT_SERVICE_HOST: &str = "127.0.0.1";
pub const DEFAULT_MONGODB_DIR: &str = "data";
pub const DEFAULT_LOCALSTACK_PORT: u16 = 4566;
pub const DEFAULT_LOCALSTACK_IMAGE: &str =
    "localstack/localstack@sha256:763947722c6c8d33d5fbf7e8d52b4bddec5be35274a0998fdc6176d733375314";
pub const DEFAULT_LOCALSTACK_CONTAINER_NAME: &str = "localstack-service";
pub const DEFAULT_MONGO_PORT: u16 = 27017;
pub const DEFAULT_MONGO_IMAGE: &str = "mongo:latest";
pub const DEFAULT_MONGO_CONTAINER_NAME: &str = "mongodb-service";
pub const MONGO_DEFAULT_DATABASE_PATH: &str = "mongodb_dump.json";
pub const DEFAULT_BINARY_DIR: &str = "../target/release";
pub const DEFAULT_DATA_DIR: &str = "./data";

pub const DEFAULT_MADARA_RPC_PORT: u16 = 9944;
pub const DEFAULT_MADARA_GATEWAY_PORT: u16 = 8080;
pub const DEFAULT_MADARA_NAME: &str = "madara";
pub const MADARA_DEFAULT_DATABASE_NAME: &str = "madara-db";
pub const DEFAULT_MADARA_BINARY_PATH: &str = "../target/release/madara";
pub const DEFAULT_MADARA_CONFIG_PATH: &str = "./config/madara.yaml";

pub const DEFAULT_MOCK_PROVER_BINARY: &str = "../target/release/mock-atlantic-server";
pub const DEFAULT_MOCK_PROVER_PORT: u16 = 8080;
pub const DEFAULT_SCRIPT_PATH: &str = "../test_utils/scripts/deploy_dummy_verifier.sh";
pub const DEFAULT_PRIVATE_KEY: &str = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
pub const DEFAULT_ANVIL_URL: &str = "http://localhost:8545";
pub const DEFAULT_MOCK_GPS_VERIFIER_PATH: &str = "test_utils/scripts/artifacts/MockGPSVerifier.sol:MockGPSVerifier";
pub const DEFAULT_VERIFIER_FILE_NAME: &str = "./data/verifier_address.txt";

pub const DEFAULT_PATHFINDER_PORT: u16 = 9545;
pub const DEFAULT_PATHFINDER_IMAGE: &str = "prkpandey942/pathfinder:549aa84_2025-05-29_appchain-vers-cons_amd";
pub const DEFAULT_PATHFINDER_CONTAINER_NAME: &str = "pathfinder-service";
