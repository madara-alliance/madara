use crate::services::{anvil::{AnvilConfig, AnvilError}, bootstrapper::{BootstrapperConfig, BootstrapperError}, localstack::{LocalstackConfig, LocalstackError}, madara::{MadaraConfig, MadaraError}, mock_prover::MockProverError, mock_verifier::{MockVerifierDeployerConfig, MockVerifierDeployerError}, mongodb::{MongoConfig, MongoError}, orchestrator::{OrchestratorConfig, OrchestratorError}, pathfinder::{PathfinderConfig, PathfinderError}};
use std::time::{Duration, Instant};
use crate::services::mock_prover::MockProverConfig;

// TODO: write layer here and use there
use crate::services::orchestrator::Layer;


// =============================================================================
// DEPENDENCIES ENUM
// =============================================================================

// Each service internally defines a sequence of dependencies that must be met before it can be started.
// If not met, the service will not be started.
pub enum Dependencies {
    AnvilIsRunning,
    DockerIsRunning,
    MongoIsRunning,
    LocalstackIsRunning,
    MadaraIsRunning,
    PathfinderIsRunning,
    OrchestratorIsRunning,
}

// =============================================================================
// ERROR TYPES
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("Anvil service error: {0}")]
    Anvil(#[from] AnvilError),
    #[error("Localstack service error: {0}")]
    Localstack(#[from] LocalstackError),
    #[error("MongoDB service error: {0}")]
    Mongo(#[from] MongoError),
    #[error("Pathfinder service error: {0}")]
    Pathfinder(#[from] PathfinderError),
    #[error("Orchestrator service error: {0}")]
    Orchestrator(#[from] OrchestratorError),
    #[error("Madara service error: {0}")]
    Madara(#[from] MadaraError),
    #[error("Bootstrapper service error: {0}")]
    Bootstrapper(#[from] BootstrapperError),
    #[error("Mock Verifier Deployer service error: {0}")]
    MockVerifierDeployer(#[from] MockVerifierDeployerError),
    #[error("Mock Prover service error: {0}")]
    MockProver(#[from] MockProverError),
    #[error("Setup timeout: {0}")]
    Timeout(String),
    #[error("Dependency validation failed: {0}")]
    DependencyFailed(String),
    #[error("Service startup failed: {0}")]
    StartupFailed(String),
    #[error("Context initialization failed: {0}")]
    ContextFailed(String),
    #[error("Other Error : {0}")]
    OtherError(String),
}

// =============================================================================
// CONSTANTS
// =============================================================================

pub const DEFAULT_BINARY_DIR: &str = "../target/release";
pub const DEFAULT_DATA_DIR: &str = "./data";


// =============================================================================
// TIMEOUT STRUCT
// =============================================================================

// Each service internally defines a sequence of dependencies that must be met before it can be started.
// If not met, the service will not be started.
#[derive(Debug, Clone)]
pub struct Timeouts {
    pub start_infrastructure_services: Duration,
    pub setup_infrastructure_services: Duration,
    pub start_l1_setup: Duration,
    pub start_l2_setup: Duration,
    pub start_full_node_syncing: Duration,
    pub start_mock_prover: Duration,
    pub start_orchestration: Duration,

    pub anvil: Duration,
    pub docker: Duration,
    pub mongo: Duration,
    pub localstack: Duration,
    pub madara: Duration,
    pub pathfinder: Duration,
    pub orchestrator: Duration,
    pub validate_dependencies: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            start_infrastructure_services: Duration::from_secs(360),
            setup_infrastructure_services: Duration::from_secs(360),
            start_l1_setup: Duration::from_secs(360),
            start_l2_setup: Duration::from_secs(1800),
            start_full_node_syncing: Duration::from_secs(300),
            start_mock_prover: Duration::from_secs(300),
            start_orchestration: Duration::from_secs(1800),

            anvil: Duration::from_secs(10),
            docker: Duration::from_secs(10),
            mongo: Duration::from_secs(10),
            localstack: Duration::from_secs(10),
            madara: Duration::from_secs(10),
            pathfinder: Duration::from_secs(10),
            orchestrator: Duration::from_secs(10),
            validate_dependencies: Duration::from_secs(10),
        }
    }
}

// =============================================================================
// CONTEXT (READ-ONLY RUNTIME CONTEXT)
// =============================================================================

// Maybe :
// layer
// madara_config {
//      port :
//      start_timeout
//      enable_stdout
//      enable_stderr
// }
// bootstrapper_config



#[derive(Debug, Clone)]
pub struct Context {

    pub layer: Layer,
    pub timeouts : Timeouts,


    // Shift all to config, context will be in-between services!
    pub localstack_config: LocalstackConfig,
    pub mongo_config: MongoConfig,
    pub orchestrator_setup_config: OrchestratorConfig,
    pub madara_config: MadaraConfig,
    pub pathfinder_config: PathfinderConfig,
    pub mock_verifier_config: MockVerifierDeployerConfig,
    pub mock_prover_config: MockProverConfig,
    pub orchestrator_run_config: OrchestratorConfig,

    pub bootstrapper_setup_l1_config: BootstrapperConfig,
    pub bootstrapper_setup_l2_config: BootstrapperConfig,
    pub anvil_config: AnvilConfig,




    pub data_directory: String,
    pub setup_timeout: Duration,
    pub wait_for_sync: bool,
    pub skip_existing_dbs: bool,
    pub db_dir_path: String,

    // // Runtime context fields
    // pub anvil_endpoint: String,
    // pub localstack_endpoint: String,
    // pub mongo_connection_string: String,
    // pub pathfinder_endpoint: String,
    // pub orchestrator_endpoint: Option<String>,
    // pub sequencer_endpoint: String,
    // pub bootstrapper_endpoint: String,
    // pub setup_start_time: Instant,
}

impl Context {
    pub fn builder() -> ContextBuilder {
        ContextBuilder::new()
    }

    /// Reader methods
    pub fn get_layer(&self) -> &Layer {
        &self.layer
    }

    /// Get Localstack Config
    pub fn get_localstack_config(&self) -> &LocalstackConfig {
        &self.localstack_config
    }

    /// Get Mongo Config
    pub fn get_mongo_config(&self) -> &MongoConfig {
        &self.mongo_config
    }


    // let anvil_db_path = format!("{}/anvil.json", self.context.db_dir_path);
    // let anvil_config = AnvilConfigBuilder::new()
    //     .port(self.context.anvil_port)
    //     .block_time(1_f64)
    //     .dump_state(anvil_db_path)
    //     .build();
    /// Get Anvil Config
    pub fn get_anvil_config(&self) -> &AnvilConfig {
        &self.anvil_config
    }


    // let madara_config = MadaraConfigBuilder::new()
    // .rpc_port(self.context.madara_port)
    // .build();
    /// Get Madara Config
    pub fn get_madara_config(&self) -> &MadaraConfig {
        &self.madara_config
    }


    // let pathfinder_config = PathfinderConfigBuilder::new().build();
    /// Get pathfinder Config
    pub fn get_pathfinder_config(&self) -> &PathfinderConfig {
        &self.pathfinder_config
    }

    // let mock_verifier_config = MockVerifierDeployerConfigBuilder::new().build();
    pub fn get_mock_verifier_config(&self) -> &MockVerifierDeployerConfig {
        &self.mock_verifier_config
    }


    // let orchestrator_setup_config = OrchestratorConfigBuilder::new()
    //     .mode(OrchestratorMode::Setup)
    //     .env_var("RUST_LOG", "info")
    //     .build();
    /// Get the Orchestrator Config
    pub fn get_orchestrator_setup_config(&self) -> &OrchestratorConfig {
        &self.orchestrator_setup_config
    }

    // let bootstrapper_l1_config = BootstrapperConfigBuilder::new()
    //     .mode(BootstrapperMode::SetupL1)
    //     .env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
    //     .env_var("ETH_RPC", "http://localhost:8545")
    //     .env_var("RUST_LOG", "info")
    //     .build();
    /// Get the Bootstrapper Setup L1 Config
    pub fn get_bootstrapper_setup_l1_config(&self) -> &BootstrapperConfig {
        &self.bootstrapper_setup_l1_config
    }

    // let bootstrapper_l2_config = BootstrapperConfigBuilder::new()
    //     .mode(BootstrapperMode::SetupL2)
    //     .config_path(DEFAULT_BOOTSTRAPPER_CONFIG)
    //     .env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
    //     .env_var("ETH_RPC", "http://localhost:8545")
    //     .env_var("RUST_LOG", "info")
    //     .build();
    /// Get the Bootstrapper Setup L2 Config
    pub fn get_bootstrapper_setup_l2_config(&self) -> &BootstrapperConfig {
        &self.bootstrapper_setup_l2_config
    }


    // let mock_prover_config = MockProverConfigBuilder::new()
    //     .port(5555)
    //     .build()?;
    /// Get the Mock Prover Config
    pub fn get_mock_prover_config(&self) -> &MockProverConfig {
        &self.mock_prover_config
    }

    // let orchestrator_setup_config = OrchestratorConfigBuilder::run_l2()
    //     .port(3000)
    //     .build();
    /// Get the Orchestrator Setup Config
    pub fn get_orchestrator_run_config(&self) -> &OrchestratorConfig {
        &self.orchestrator_run_config
    }


    /// Get Timeout Config
    pub fn get_timeouts(&self) -> &Timeouts {
        &self.timeouts
    }




}

// =============================================================================
// CONTEXT BUILDER (CONFIGURATION/SETTER METHODS)
// =============================================================================

pub struct ContextBuilder {
    context: Context,
}

impl ContextBuilder {
    pub fn new() -> Self {
        // TODO: We've defined defaults ports in each servces' config use that
        let anvil_port = 8545;
        let localstack_port = 4566;
        let mongo_port = 27017;
        let pathfinder_port = 9545;
        let orchestrator_port: Option<u16> = None;
        let madara_port = 9944;
        let bootstrapper_port = 9945;
        let data_directory = "/tmp/madara-setup".to_string();

        let context = Context {
            // Configuration fields
            layer: Layer::L2,
            ethereum_api_key: String::new(),
            anvil_port,
            localstack_port,
            mongo_port,
            pathfinder_port,
            orchestrator_port,
            madara_port,
            bootstrapper_port,
            data_directory: data_directory.clone(),
            setup_timeout: Duration::from_secs(300), // 5 minutes
            wait_for_sync: true,
            skip_existing_dbs: false,
            db_dir_path: DEFAULT_DATA_DIR.to_string(),

            // Runtime context fields
            anvil_endpoint: format!("http://127.0.0.1:{}", anvil_port),
            localstack_endpoint: format!("http://127.0.0.1:{}", localstack_port),
            mongo_connection_string: format!("mongodb://127.0.0.1:{}/madara", mongo_port),
            pathfinder_endpoint: format!("http://127.0.0.1:{}", pathfinder_port),
            orchestrator_endpoint: orchestrator_port.map(|port| format!("http://127.0.0.1:{}", port)),
            sequencer_endpoint: format!("http://127.0.0.1:{}", madara_port),
            bootstrapper_endpoint: format!("http://127.0.0.1:{}", bootstrapper_port),
            setup_start_time: Instant::now(),
        };

        Self { context }
    }

    // Setter methods (builder pattern)
    pub fn layer(mut self, layer: Layer) -> Self {
        self.context.layer = layer;
        self
    }

    pub fn ethereum_api_key(mut self, api_key: String) -> Self {
        self.context.ethereum_api_key = api_key;
        self
    }

    pub fn anvil_port(mut self, port: u16) -> Self {
        self.context.anvil_port = port;
        self.context.anvil_endpoint = format!("http://127.0.0.1:{}", port);
        self
    }

    pub fn localstack_port(mut self, port: u16) -> Self {
        self.context.localstack_port = port;
        self.context.localstack_endpoint = format!("http://127.0.0.1:{}", port);
        self
    }

    pub fn mongo_port(mut self, port: u16) -> Self {
        self.context.mongo_port = port;
        self.context.mongo_connection_string = format!("mongodb://127.0.0.1:{}/madara", port);
        self
    }

    pub fn pathfinder_port(mut self, port: u16) -> Self {
        self.context.pathfinder_port = port;
        self.context.pathfinder_endpoint = format!("http://127.0.0.1:{}", port);
        self
    }

    pub fn orchestrator_port(mut self, port: Option<u16>) -> Self {
        self.context.orchestrator_port = port;
        self.context.orchestrator_endpoint = port.map(|p| format!("http://127.0.0.1:{}", p));
        self
    }

    pub fn madara_port(mut self, port: u16) -> Self {
        self.context.madara_port = port;
        self.context.sequencer_endpoint = format!("http://127.0.0.1:{}", port);
        self
    }

    pub fn bootstrapper_port(mut self, port: u16) -> Self {
        self.context.bootstrapper_port = port;
        self.context.bootstrapper_endpoint = format!("http://127.0.0.1:{}", port);
        self
    }

    pub fn data_directory(mut self, directory: String) -> Self {
        self.context.data_directory = directory;
        self
    }

    pub fn setup_timeout(mut self, timeout: Duration) -> Self {
        self.context.setup_timeout = timeout;
        self
    }

    pub fn wait_for_sync(mut self, wait: bool) -> Self {
        self.context.wait_for_sync = wait;
        self
    }

    pub fn skip_existing_dbs(mut self, skip: bool) -> Self {
        self.context.skip_existing_dbs = skip;
        self
    }

    pub fn db_dir_path(mut self, path: String) -> Self {
        self.context.db_dir_path = path;
        self
    }

    pub fn build(self) -> Context {
        self.context
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
