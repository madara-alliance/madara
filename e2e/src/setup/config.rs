use crate::services::localstack::LocalstackConfigBuilder;
use crate::services::mock_prover::MockProverConfig;
use crate::services::mongodb::MongoConfigBuilder;
use crate::services::server::ServerError;
use crate::services::{
    anvil::{AnvilConfig, AnvilConfigBuilder, AnvilError},
    bootstrapper::{BootstrapperConfig, BootstrapperConfigBuilder, BootstrapperError, BootstrapperMode},
    helpers::{get_database_path, get_free_port},
    localstack::{LocalstackConfig, LocalstackError},
    madara::{MadaraConfig, MadaraConfigBuilder, MadaraError},
    mock_prover::{MockProverConfigBuilder, MockProverError},
    mock_verifier::{MockVerifierDeployerConfig, MockVerifierDeployerConfigBuilder, MockVerifierDeployerError},
    mongodb::{MongoConfig, MongoError},
    orchestrator::{OrchestratorConfig, OrchestratorConfigBuilder, OrchestratorError, OrchestratorMode},
    pathfinder::{PathfinderConfig, PathfinderConfigBuilder, PathfinderError},
};
use std::time::Duration;
use crate::services::constants::*;
use crate::services::orchestrator::Layer;

#[derive(Debug, PartialEq, serde::Serialize)]
pub enum DBState {
    ReadyToUse,
    Locked,
    NotReady,
    Error,
}

impl From<String> for DBState {
    fn from(s: String) -> Self {
        match s.as_str() {
            "READY" => DBState::ReadyToUse,
            "NOT_READY" => DBState::NotReady,
            "LOCKED" => DBState::Locked,
            _ => DBState::Error,
        }
    }
}

// =============================================================================
// ERROR TYPES
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
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
    #[error("IO Error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Other Error : {0}")]
    OtherError(String),
}

// =============================================================================
// TIMEOUT STRUCT
// =============================================================================

// Each service internally defines a sequence of dependencies that must be met before it can be started.
// If not met, the service will not be started.
#[derive(Debug, Clone)]
pub struct Timeouts {
    pub validate_dependencies: Duration,
    pub start_infrastructure_services: Duration,
    pub setup_localstack_infrastructure_services: Duration,
    pub setup_mongodb_infrastructure_services: Duration,
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
            setup_localstack_infrastructure_services: Duration::from_secs(360),
            setup_mongodb_infrastructure_services: Duration::from_secs(360),
            start_l1_setup: Duration::from_secs(360),
            start_l2_setup: Duration::from_secs(1800),
            start_full_node_syncing: Duration::from_secs(300),
            start_mock_prover: Duration::from_secs(300),
            start_orchestration: Duration::from_secs(10000),
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
    pub timeouts: Timeouts,

    // Individual Component Configurations
    pub anvil_config: AnvilConfig,
    pub localstack_config: LocalstackConfig,
    pub mongo_config: MongoConfig,
    pub orchestrator_setup_config: OrchestratorConfig,
    pub madara_config: MadaraConfig,
    pub pathfinder_config: PathfinderConfig,
    pub mock_verifier_deployer_config: MockVerifierDeployerConfig,
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
            mock_verifier_deployer_config: MockVerifierDeployerConfig::default(),
            mock_prover_config: MockProverConfig::default(),
            orchestrator_run_config: OrchestratorConfig::default(),
            bootstrapper_setup_l1_config: BootstrapperConfig::default(),
            bootstrapper_setup_l2_config: BootstrapperConfig::default(),
        }
    }
}

impl SetupConfig {
    /// Get the builder
    pub fn builder(&self) -> SetupConfigBuilder {
        SetupConfigBuilder::new(Some(self.clone()))
    }

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

    /// Get Anvil Config
    pub fn get_anvil_config(&self) -> &AnvilConfig {
        &self.anvil_config
    }

    /// Get Madara Config
    pub fn get_madara_config(&self) -> &MadaraConfig {
        &self.madara_config
    }

    /// Get pathfinder Config
    pub fn get_pathfinder_config(&self) -> &PathfinderConfig {
        &self.pathfinder_config
    }

    /// Get Mock Verifier Deployer Config
    pub fn get_mock_verifier_deployer_config(&self) -> &MockVerifierDeployerConfig {
        &self.mock_verifier_deployer_config
    }

    /// Get the Orchestrator Config
    pub fn get_orchestrator_setup_config(&self) -> &OrchestratorConfig {
        &self.orchestrator_setup_config
    }

    /// Get the Bootstrapper Setup L1 Config
    pub fn get_bootstrapper_setup_l1_config(&self) -> &BootstrapperConfig {
        &self.bootstrapper_setup_l1_config
    }

    /// Get the Bootstrapper Setup L2 Config
    pub fn get_bootstrapper_setup_l2_config(&self) -> &BootstrapperConfig {
        &self.bootstrapper_setup_l2_config
    }

    /// Get the Mock Prover Config
    pub fn get_mock_prover_config(&self) -> &MockProverConfig {
        &self.mock_prover_config
    }

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
    pub fn new(config: Option<SetupConfig>) -> Self {
        Self { config: config.unwrap_or_else(SetupConfig::default) }
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
    pub fn mock_verifier_deployer_config(mut self, mock_verifier_deployer_config: MockVerifierDeployerConfig) -> Self {
        self.config.mock_verifier_deployer_config = mock_verifier_deployer_config;
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

    pub fn build_l2_setup_config(self) -> Result<SetupConfig, SetupError> {
        let mongodb_config = MongoConfigBuilder::new().port(get_free_port()?).logs((false, true)).build();

        let localstack_port = get_free_port()?;
        let localstack_host = format!("{}:{}", DEFAULT_SERVICE_HOST, localstack_port);
        let localstack_config = LocalstackConfigBuilder::new()
            .port(localstack_port)
            .logs((false, true))
            .env_var("LOCALSTACK_HOST", localstack_host)
            .build();

        let orchestrator_setup_config = OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .env_var("AWS_ENDPOINT_URL", localstack_config.endpoint())
            .logs((true, true))
            .build();

        let anvil_config = AnvilConfigBuilder::new()
            .port(get_free_port()?)
            .dump_state(get_database_path(DATA_DIR, ANVIL_DATABASE_FILE))
            .logs((false, true))
            .build();

        let mock_verifier_deployer_config =
            MockVerifierDeployerConfigBuilder::new().l1_url(anvil_config.endpoint()).logs((true, true)).build();

        let bootstrapper_l1_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL1)
            .config_path(BOOTSTRAPPER_CONFIG)
            .env_var("ETH_RPC", anvil_config.endpoint().as_str())
            .env_var("ETH_PRIVATE_KEY", ANVIL_PRIVATE_KEY)
            .logs((true, true))
            .build();

        let madara_config = MadaraConfigBuilder::new()
            .rpc_port(get_free_port()?)
            .rpc_admin_port(get_free_port()?)
            .gateway_port(get_free_port()?)
            .database_path(get_database_path(DATA_DIR, MADARA_DATABASE_DIR))
            .l1_endpoint(Some(anvil_config.endpoint()))
            .logs((true, true))
            .build();

        let bootstrapper_l2_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL2)
            .config_path(BOOTSTRAPPER_CONFIG)
            .timeout(BOOTSTRAPPER_SETUP_L2_TIMEOUT.clone())
            .env_var("ETH_RPC", anvil_config.endpoint().as_str())
            .env_var("ETH_PRIVATE_KEY", ANVIL_PRIVATE_KEY)
            .env_var("ROLLUP_SEQ_URL", madara_config.rpc_endpoint().as_str())
            .env_var("ROLLUP_DECLARE_V0_SEQ_URL", madara_config.rpc_admin_endpoint().as_str())
            .logs((true, true))
            .build();

        let pathfinder_config = PathfinderConfigBuilder::new()
            .port(get_free_port()?)
            .gateway_url(Some(madara_config.gateway_endpoint()))
            .feeder_gateway_url(Some(madara_config.feeder_gateway_endpoint()))
            .logs((true, true))
            .build();

        let mock_prover_config = MockProverConfigBuilder::new().port(get_free_port()?).logs((true, true)).build();

        let mongodb_config = MongoConfigBuilder::new()
            .port(get_free_port()?)
            .logs((false, true))
            .build();

        let localstack_port = get_free_port()?;
        let localstack_host = format!("localhost:{}", localstack_port);
        let localstack_config = LocalstackConfigBuilder::new()
            .port(localstack_port)
            .logs((false, true))
            .env_var("LOCALSTACK_HOST", localstack_host)
            .build();

        let orchestrator_setup_config = OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .env_var("MADARA_ORCHESTRATOR_AWS_PREFIX", test_name)
            .env_var("AWS_ENDPOINT_URL", localstack_config.endpoint())
            .logs((true, true))
            .build();

        let orchestrator_run_config = OrchestratorConfigBuilder::run_l2()
            .port(get_free_port()?)
            .da_on_ethereum(true)
            .settle_on_ethereum(true)
            .ethereum_rpc_url(anvil_config.endpoint())
            .mongodb(true)
            .mongodb_connection_url(mongodb_config.endpoint())
            .atlantic_service_url(mock_prover_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_MADARA_RPC_URL", pathfinder_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_RPC_FOR_SNOS", pathfinder_config.endpoint())
            .env_var("AWS_ENDPOINT_URL", localstack_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", anvil_config.endpoint().as_str())
            .env_var("MADARA_ORCHESTRATOR_ETHEREUM_DA_RPC_URL", anvil_config.endpoint().as_str())
            .env_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", anvil_config.endpoint().as_str())
            .logs((true, true))
            .build();

        let sconfig = self
            .anvil_config(anvil_config)
            .madara_config(madara_config)
            .pathfinder_config(pathfinder_config)
            .mock_verifier_deployer_config(mock_verifier_deployer_config)
            .orchestrator_setup_config(orchestrator_setup_config)
            .bootstrapper_setup_l1_config(bootstrapper_l1_config)
            .bootstrapper_setup_l2_config(bootstrapper_l2_config)
            .mock_prover_config(mock_prover_config)
            .mongo_config(mongodb_config)
            .localstack_config(localstack_config)
            .orchestrator_run_config(orchestrator_run_config)
            .build();

        Ok(sconfig)
    }

    pub fn test_config_l2(self, test_name: &str) -> Result<SetupConfig, SetupError> {
        let mongodb_config = MongoConfigBuilder::new().port(get_free_port()?).logs((false, true)).build();

        let localstack_port = get_free_port()?;
        let localstack_host = format!("{}:{}", DEFAULT_SERVICE_HOST, localstack_port);
        let localstack_config = LocalstackConfigBuilder::new()
            .port(localstack_port)
            .logs((false, true))
            .env_var("LOCALSTACK_HOST", localstack_host)
            .build();

        let orchestrator_setup_config = OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .env_var("AWS_ENDPOINT_URL", localstack_config.endpoint())
            .logs((true, true))
            .build();

        let anvil_config = AnvilConfigBuilder::new()
            .port(get_free_port()?)
            .load_state(get_database_path(test_name, ANVIL_DATABASE_FILE))
            .logs((true, true))
            .build();

        let mock_verifier_deployer_config =
            MockVerifierDeployerConfigBuilder::new().l1_url(anvil_config.endpoint()).logs((true, true)).build();

        let bootstrapper_l1_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL1)
            .env_var("ETH_RPC", anvil_config.endpoint().as_str())
            .logs((true, true))
            .build();

        let madara_config = MadaraConfigBuilder::new()
            .rpc_port(get_free_port()?)
            .rpc_admin_port(get_free_port()?)
            .gateway_port(get_free_port()?)
            .database_path(get_database_path(test_name, MADARA_DATABASE_DIR))
            .l1_endpoint(Some(anvil_config.endpoint()))
            .logs((true, true))
            .build();

        let bootstrapper_l2_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL2)
            .config_path(BOOTSTRAPPER_CONFIG)
            .env_var("ETH_RPC", anvil_config.endpoint().as_str())
            .logs((true, true))
            .build();

        let pathfinder_config = PathfinderConfigBuilder::new()
            .port(get_free_port()?)
            .gateway_url(Some(madara_config.gateway_endpoint()))
            .feeder_gateway_url(Some(madara_config.feeder_gateway_endpoint()))
            .logs((true, true))
            .build();

        let mock_prover_config = MockProverConfigBuilder::new().port(get_free_port()?).logs((true, true)).build();

        let orchestrator_run_config = OrchestratorConfigBuilder::run_l2()
            .port(get_free_port()?)
            .da_on_ethereum(true)
            .settle_on_ethereum(true)
            .ethereum_rpc_url(anvil_config.endpoint())
            .mongodb(true)
            .mongodb_connection_url(mongodb_config.endpoint())
            .atlantic_service_url(mock_prover_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_MADARA_RPC_URL", pathfinder_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_RPC_FOR_SNOS", pathfinder_config.endpoint())
            .env_var("AWS_ENDPOINT_URL", localstack_config.endpoint())
            .env_var("MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL", anvil_config.endpoint().as_str())
            .env_var("MADARA_ORCHESTRATOR_ETHEREUM_DA_RPC_URL", anvil_config.endpoint().as_str())
            .env_var("MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL", anvil_config.endpoint().as_str())
            .logs((true, true))
            .build();

        let sconfig = self
            .anvil_config(anvil_config)
            .madara_config(madara_config)
            .pathfinder_config(pathfinder_config)
            .mock_verifier_deployer_config(mock_verifier_deployer_config)
            .orchestrator_setup_config(orchestrator_setup_config)
            .bootstrapper_setup_l1_config(bootstrapper_l1_config)
            .bootstrapper_setup_l2_config(bootstrapper_l2_config)
            .mock_prover_config(mock_prover_config)
            .mongo_config(mongodb_config)
            .localstack_config(localstack_config)
            .orchestrator_run_config(orchestrator_run_config)
            .build();

        Ok(sconfig)
    }
}

impl Default for SetupConfigBuilder {
    fn default() -> Self {
        Self::new(None)
    }
}
