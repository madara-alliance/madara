use crate::services::{anvil::{AnvilConfig, AnvilError}, bootstrapper::{BootstrapperConfig, BootstrapperError}, localstack::{LocalstackConfig, LocalstackError}, madara::{MadaraConfig, MadaraError}, mock_prover::MockProverError, mock_verifier::{MockVerifierDeployerConfig, MockVerifierDeployerError}, mongodb::{MongoConfig, MongoError}, orchestrator::{OrchestratorConfig, OrchestratorError}, pathfinder::{PathfinderConfig, PathfinderError}};
use std::time::Duration;
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
    pub validate_dependencies: Duration,
    pub start_infrastructure_services: Duration,
    pub setup_infrastructure_services: Duration,
    pub start_l1_setup: Duration,
    pub start_l2_setup: Duration,
    pub start_full_node_syncing: Duration,
    pub start_mock_prover: Duration,
    pub start_orchestration: Duration,
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
            validate_dependencies: Duration::from_secs(10),
        }
    }
}

// =============================================================================
// CONFIG (READ-ONLY RUNTIME CONTEXT)
// =============================================================================


#[derive(Debug, Clone)]
pub struct SetupConfig {
    // General Configurations
    pub layer: Layer,
    pub timeouts : Timeouts,

    // Individual Component Configurations
    pub anvil_config: AnvilConfig,
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
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            layer: Layer::L2,
            timeouts: Timeouts::default(),
            anvil_config: AnvilConfig::default(),
            localstack_config: LocalstackConfig::default(),
            mongo_config: MongoConfig::default(),
            orchestrator_setup_config: OrchestratorConfig::default(),
            madara_config: MadaraConfig::default(),
            pathfinder_config: PathfinderConfig::default(),
            mock_verifier_config: MockVerifierDeployerConfig::default(),
            mock_prover_config: MockProverConfig::default(),
            orchestrator_run_config: OrchestratorConfig::default(),
            bootstrapper_setup_l1_config: BootstrapperConfig::default(),
            bootstrapper_setup_l2_config: BootstrapperConfig::default(),
        }
    }
}

impl SetupConfig {
    /// Get Layer Config
    pub fn get_layer(&self) -> &Layer {
        &self.layer
    }

    /// Get Timeout Config
    pub fn get_timeouts(&self) -> &Timeouts {
        &self.timeouts
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

}

// =============================================================================
// CONTEXT BUILDER (CONFIGURATION/SETTER METHODS)
// =============================================================================

#[derive(Debug, Clone)]
pub struct SetupConfigBuilder {
    config: SetupConfig,
}

impl SetupConfigBuilder {
    pub fn new() -> Self {
        Self {
            config: SetupConfig::default(),
        }
    }

    /// Set Layer
    pub fn layer(mut self, layer: Layer) -> Self {
        self.config.layer = layer;
        self
    }

    /// Set Timeouts
    pub fn timeouts(mut self, timeouts: Timeouts) -> Self {
        self.config.timeouts = timeouts;
        self
    }

    /// Set the Anvil Config
    pub fn anvil_config(mut self, anvil_config: AnvilConfig) -> Self {
        self.config.anvil_config = anvil_config;
        self
    }

    /// Set the Localstack Config
    pub fn localstack_config(mut self, localstack_config: LocalstackConfig) -> Self {
        self.config.localstack_config = localstack_config;
        self
    }

    /// Set the Mongo Config
    pub fn mongo_config(mut self, mongo_config: MongoConfig) -> Self {
        self.config.mongo_config = mongo_config;
        self
    }


    /// Set the Orchestrator Run Config
    pub fn orchestrator_run_config(mut self, orchestrator_run_config: OrchestratorConfig) -> Self {
        self.config.orchestrator_run_config = orchestrator_run_config;
        self
    }

    /// Set the orchestrator setup config
    pub fn orchestrator_setup_config(mut self, orchestrator_setup_config: OrchestratorConfig) -> Self {
        self.config.orchestrator_setup_config = orchestrator_setup_config;
        self
    }


    /// Set the Madara Config
    pub fn madara_config(mut self, madara_config: MadaraConfig) -> Self {
        self.config.madara_config = madara_config;
        self
    }

    /// Set the Pathfinder Config
    pub fn pathfinder_config(mut self, pathfinder_config: PathfinderConfig) -> Self {
        self.config.pathfinder_config = pathfinder_config;
        self
    }

    /// Set the Mock Verifier Config
    pub fn mock_verifier_config(mut self, mock_verifier_config: MockVerifierDeployerConfig) -> Self {
        self.config.mock_verifier_config = mock_verifier_config;
        self
    }

    /// Set the Mock Prover Config
    pub fn mock_prover_config(mut self, mock_prover_config: MockProverConfig) -> Self {
        self.config.mock_prover_config = mock_prover_config;
        self
    }

    /// Set the Bootstrapper Setup L1 Config
    pub fn bootstrapper_setup_l1_config(mut self, bootstrapper_setup_l1_config: BootstrapperConfig) -> Self {
        self.config.bootstrapper_setup_l1_config = bootstrapper_setup_l1_config;
        self
    }

    /// Set the Bootstrapper Setup L2 Config
    pub fn bootstrapper_setup_l2_config(mut self, bootstrapper_setup_l2_config: BootstrapperConfig) -> Self {
        self.config.bootstrapper_setup_l2_config = bootstrapper_setup_l2_config;
        self
    }

    pub fn build(self) -> SetupConfig {
        self.config
    }
}

impl Default for SetupConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}
