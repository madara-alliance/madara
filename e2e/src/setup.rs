use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

use::std::path::Path;
// Import all the services we've created
use crate::services::anvil::{AnvilConfigBuilder, AnvilError, AnvilService};
use crate::services::bootstrapper::{BootstrapperConfigBuilder, BootstrapperError, BootstrapperMode, BootstrapperService};
use crate::services::docker::DockerServer;
use crate::services::mock_verifier::{MockVerifierDeployerConfigBuilder, MockVerifierDeployerError, MockVerifierDeployerService};
use crate::services::pathfinder::DEFAULT_PATHFINDER_IMAGE;
use crate::services::mongodb::MONGO_DEFAULT_DATABASE_PATH;
use crate::services::bootstrapper::DEFAULT_BOOTSTRAPPER_CONFIG;
use crate::services::bootstrapper::BOOTSTRAPPER_DEFAULT_ADDRESS_PATH;
use crate::services::anvil::ANVIL_DEFAULT_DATABASE_NAME;
use crate::services::madara::MADARA_DEFAULT_DATABASE_NAME;
use crate::services::mongodb::DEFAULT_MONGO_IMAGE;
use crate::services::localstack::DEFAULT_LOCALSTACK_IMAGE;
use crate::services::localstack::{ LocalstackError, LocalstackService, LocalstackConfigBuilder};
use crate::services::madara::{ MadaraError, MadaraService, MadaraConfigBuilder};
use crate::services::mongodb::{ MongoConfigBuilder, MongoError, MongoService};
use crate::services::orchestrator::{
    Layer, OrchestratorError, OrchestratorMode, OrchestratorService, OrchestratorConfigBuilder
};
use crate::services::mock_prover::{MockProverConfigBuilder, MockProverService, MockProverError};
use crate::services::pathfinder::{PathfinderConfigBuilder, PathfinderError, PathfinderService};
use crate::services::helpers::NodeRpcMethods;

// TODO:
// Ensure to have all the binaries built
// Ensure to build the mock-verifier as well
// CARGO_TARGET_DIR=target cargo build --manifest-path test_utils/crates/mock-atlantic-server/Cargo.toml  --bin mock-atlantic-server --release

// TODO: Implemented this to always be from the root, and not relative
const DEFAULT_BINARY_DIR: &str = "../target/release";
const DEFAULT_DATA_DIR: &str = "./data";

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

#[derive(Debug, Clone)]
pub struct SetupConfig {
    pub layer: Layer,
    pub ethereum_api_key: String,
    pub anvil_port: u16,
    pub localstack_port: u16,
    pub mongo_port: u16,
    pub pathfinder_port: u16,
    pub orchestrator_port: Option<u16>,
    pub madara_port: u16,
    pub bootstrapper_port: u16,
    pub data_directory: String,
    pub setup_timeout: Duration,
    pub wait_for_sync: bool,
    pub skip_existing_dbs: bool,
    pub db_dir_path: String
}

impl Default for SetupConfig {
    fn default() -> Self {
        Self {
            layer: Layer::L2,
            ethereum_api_key: String::new(),
            anvil_port: 8545,
            localstack_port: 4566,
            mongo_port: 27017,
            pathfinder_port: 9545,
            orchestrator_port: None,
            madara_port: 9944,
            bootstrapper_port: 9945,
            data_directory: "/tmp/madara-setup".to_string(),
            setup_timeout: Duration::from_secs(300), // 5 minutes
            wait_for_sync: true,
            skip_existing_dbs: false,
            db_dir_path: DEFAULT_DATA_DIR.to_string()
        }
    }
}


// Setup can be sub-divided into to parts :
// 1. Setup a new chain
// 2. Pick up from a pre-existing chain

#[derive(Debug, Clone)]
pub struct Context {
    pub layer: Layer,
    pub anvil_endpoint: String,
    pub localstack_endpoint: String,
    pub mongo_connection_string: String,
    pub pathfinder_endpoint: String,
    pub orchestrator_endpoint: Option<String>,
    pub sequencer_endpoint: String,
    pub bootstrapper_endpoint: String,
    pub data_directory: String,
    pub setup_start_time: std::time::Instant,
}

impl Context {
    pub fn new(config: &SetupConfig) -> Self {
        Self {
            layer: config.layer.clone(),
            anvil_endpoint: format!("http://127.0.0.1:{}", config.anvil_port),
            localstack_endpoint: format!("http://127.0.0.1:{}", config.localstack_port),
            mongo_connection_string: format!("mongodb://127.0.0.1:{}/madara", config.mongo_port),
            pathfinder_endpoint: format!("http://127.0.0.1:{}", config.pathfinder_port),
            orchestrator_endpoint: config.orchestrator_port.map(|port| format!("http://127.0.0.1:{}", port)),
            sequencer_endpoint: format!("http://127.0.0.1:{}", config.madara_port),
            bootstrapper_endpoint: format!("http://127.0.0.1:{}", config.bootstrapper_port),
            data_directory: config.data_directory.clone(),
            setup_start_time: std::time::Instant::now(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.setup_start_time.elapsed()
    }
}

pub struct Setup {
    pub anvil: Option<AnvilService>,
    pub localstack: Option<LocalstackService>,
    pub mongo: Option<MongoService>,
    pub pathfinder: Option<PathfinderService>,
    pub orchestrator: Option<OrchestratorService>,
    pub madara: Option<MadaraService>,
    pub bootstrapper: Option<BootstrapperService>,
    pub context: Arc<Context>,
    pub config: SetupConfig,
}

// Dependencies that each task enlists that we validate!
pub enum Dependencies{
    AnvilIsRunning,
    DockerIsRunning,
    MongoIsRunning,
    LocalstackIsRunning,
    LocalstackIsSetup,
    BootstrapperL1Setup,
    BootstrapperL2Setup,
    MadaraIsRunning,
    MadaraIsSetup,
    PathfinderIsRunning,
    OrchestratorIsRunning
}

// TODO: need a validation on weather the binaries are available or not!

impl Setup {
    /// Create a new setup instance
    pub fn new(config: SetupConfig) -> Result<Self, SetupError> {
        let context = Arc::new(Context::new(&config));

        Ok(Self {
            anvil: None,
            localstack: None,
            mongo: None,
            pathfinder: None,
            orchestrator: None,
            madara: None,
            bootstrapper: None,
            context,
            config,
        })
    }

    /// Run the complete setup process
    pub async fn setup(&mut self) -> Result<(), SetupError> {
        println!("🚀 Starting Madara Setup for {:?} layer...", self.config.layer);

        // BLOCKING PROCESS
        // Step 1 : Validate dependencies within timeout
        // Anvil should be installed
        // Docker should be present and running
        timeout(self.config.setup_timeout, async {
            self.validate_dependencies().await?;
            Ok::<(), SetupError>(())
        }).await
        .map_err(|_| SetupError::Timeout("Setup process timed out".to_string()))??;


        // Step 2 : Check for existing chain state
        // Decision : if state exists, skip setting up new chain and start servers with existing state
        // else : if state does not exist, setup new chain and start servers with new state

        let state_exists = self.check_existing_chain_state().await?;

        if state_exists {
            println!("✅ Chain state exists, starting servers...");
            // self.start_existing_chain().await?;
        } else {
            println!("❌ Chain state does not exist, setting up new chain...");
            self.setup_new_chain().await?;
        }

        Ok(())
    }

    /// Validate all required dependencies
    async fn validate_dependencies(&self) -> Result<(), SetupError> {
        println!("🔍 Validating dependencies...");

        let mut join_set = JoinSet::new();

        // First, validate basic dependencies that don't depend on Docker
        // Anvil
        join_set.spawn(async {
            let result = std::process::Command::new("anvil").arg("--version").output();
            if result.is_err() {
                return Err(SetupError::DependencyFailed("❌ Anvil not found".to_string()));
            }
            println!("✅ Anvil is available");
            Ok(())
        });

        // Forge
        join_set.spawn(async {
            let result = std::process::Command::new("forge").arg("--version").output();
            if result.is_err() {
                return Err(SetupError::DependencyFailed("❌ Forge not found".to_string()));
            }
            println!("✅ Forge is available");
            Ok(())
        });

        // Validate Docker availability
        join_set.spawn(async {
            if !DockerServer::is_docker_running() {
                println!("❌ Docker is NOT running");
                return Err(SetupError::DependencyFailed("Docker not running".to_string()));
            }
            println!("✅ Docker is running");
            Ok(())
        });

        // Wait for basic validations to complete
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        // Now that we know Docker is available, pull required images concurrently
        println!("📦 Pulling required Docker images...");


        // TODO: I believe we can create an array of images and use that,
        // it will reduce the error handling below to one time

        // Pull mongo:latest
        join_set.spawn(async {
            println!("📦 Pulling mongo:latest...");
            let mongo_result = tokio::process::Command::new("docker")
                .args(["pull", DEFAULT_MONGO_IMAGE])
                .output()
                .await;

            match mongo_result {
                Ok(output) if output.status.success() => {
                    println!("✅ Successfully pulled mongo:latest");
                    Ok(())
                }
                Ok(output) => {
                    let error_msg = String::from_utf8_lossy(&output.stderr);
                    Err(SetupError::DependencyFailed(
                        format!("Failed to pull mongo:latest: {}", error_msg)
                    ))
                }
                Err(e) => {
                    Err(SetupError::DependencyFailed(
                        format!("Failed to execute docker pull mongo:latest: {}", e)
                    ))
                }
            }
        });

        // Pull localstack image
        join_set.spawn(async {
            println!("📦 Pulling localstack/localstack...");
            let localstack_result = tokio::process::Command::new("docker")
                .args([
                    "pull",
                    DEFAULT_LOCALSTACK_IMAGE
                ])
                .output()
                .await;

            match localstack_result {
                Ok(output) if output.status.success() => {
                    println!("✅ Successfully pulled localstack/localstack");
                    Ok(())
                }
                Ok(output) => {
                    let error_msg = String::from_utf8_lossy(&output.stderr);
                    Err(SetupError::DependencyFailed(
                        format!("Failed to pull localstack/localstack: {}", error_msg)
                    ))
                }
                Err(e) => {
                    Err(SetupError::DependencyFailed(
                        format!("Failed to execute docker pull localstack/localstack: {}", e)
                    ))
                }
            }
        });

        // Pull pathfinder image
        join_set.spawn(async {
            println!("📦 Pulling pathfinder image...");
            let localstack_result = tokio::process::Command::new("docker")
                .args([
                    "pull",
                    DEFAULT_PATHFINDER_IMAGE
                ])
                .output()
                .await;

            match localstack_result {
                Ok(output) if output.status.success() => {
                    println!("✅ Successfully pulled pathfinder image");
                    Ok(())
                }
                Ok(output) => {
                    let error_msg = String::from_utf8_lossy(&output.stderr);
                    Err(SetupError::DependencyFailed(
                        format!("Failed to pull pathfinder image: {}", error_msg)
                    ))
                }
                Err(e) => {
                    Err(SetupError::DependencyFailed(
                        format!("Failed to execute docker pull pathfinder image: {}", e)
                    ))
                }
            }
        });

        // Wait for image pulls to complete
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        println!("✅ All dependencies validated");
        Ok(())
    }

    /// Check if database is exisitng or not in the Default Data Directory
    async fn check_existing_chain_state(&self) -> Result<bool, SetupError> {
        println!("🗄️  Checking existing databases...");

        let data_dir = Path::new(DEFAULT_DATA_DIR);

        // Check for required files/directories
        let mongodb_dump_exists = data_dir.join(MONGO_DEFAULT_DATABASE_PATH).exists();
        let anvil_json_exists = data_dir.join(ANVIL_DEFAULT_DATABASE_NAME).exists();
        let madara_db_exists = data_dir.join(MADARA_DEFAULT_DATABASE_NAME).exists();
        let address_json_exists = data_dir.join(BOOTSTRAPPER_DEFAULT_ADDRESS_PATH).exists();

        let all_exist = mongodb_dump_exists && anvil_json_exists && madara_db_exists && address_json_exists;

        // Do we wanna implement half state ?
        // i.e if addresses.json and anvil.json exists then we are done with l1 setup

        // TODO: what if some exit ?
        // Clean and move on!

        Ok(all_exist)
    }

    /// Starts afresh chain
    async fn setup_new_chain(&mut self) -> Result<(), SetupError> {

        // Start infrastructure services
        timeout(Duration::from_secs(360), async {
            self.start_infrastructure_services().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start infrastructure services timed out".to_string()))??;


        // Setup infrastructure services
        timeout(Duration::from_secs(360), async {
            self.setup_infrastructure_services().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup infrastructure services timed out".to_string()))??;


        // Setup L1 Chain
        timeout(Duration::from_secs(70), async {
            self.start_l1_setup().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L2 Chain timed out".to_string()))??;


        // Setup L2 Chain
        timeout(Duration::from_secs(1800), async {
            self.start_l2_setup().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L2 Chain timed out".to_string()))??;


        // Start Full Node Syncing
        timeout(Duration::from_secs(300), async {
            self.start_full_node_syncing().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Full Node Syncing timed out".to_string()))??;

        // Start Mock Proving Service
        timeout(Duration::from_secs(500), async {
            self.start_mock_prover().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Mock Proving Service timed out".to_string()))??;

        // Start Orchestrator Service
        timeout(Duration::from_secs(500), async {
            self.start_orchestration().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Orchestrator Service timed out".to_string()))??;

        println!("✅ Setup completed successfully in {:?}", self.context.elapsed());

        Ok(())

    }


    /// Start L1 setup (Anvil, Bootstrapper)
    async fn start_l1_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L1 setup...");

        // No need to do load state on setup!
        // Only dump state on shutdown!

        // Anvil db path :
        let anvil_db_path = format!("{}/anvil.json", self.config.db_dir_path.clone());

        let anvil_config = AnvilConfigBuilder::new()
            .port(self.config.anvil_port.clone())
            .block_time(1_f64)
            .dump_state(anvil_db_path).build();

        // Create async closures that DON'T borrow self
        let start_anvil = async move {
            let service = AnvilService::start(anvil_config).await?;
            println!("✅ Anvil started on {}", service.endpoint());
            Ok::<AnvilService, SetupError>(service)
        };
        let anvil_service = start_anvil.await?;

        // Assign the services
        self.anvil = Some(anvil_service);


        println!("🧑‍💻 Starting mock verifier deployer");

        // Deploying verifier contract

        let mock_verifier_config = MockVerifierDeployerConfigBuilder::new().build();
        let status_1 = MockVerifierDeployerService::run(mock_verifier_config).await?;
        println!("🥳 Mock verifier deployer finished with {}", status_1);

        // Read from ./data/verifier_address.txt
        let verifier_address = std::fs::read_to_string("./data/verifier_address.txt")
            .map_err(|e| SetupError::OtherError(format!("Failed to read verifier address file: {}", e)))?
            .trim()
            .to_string();

        // Put it to ./config/bootstrapper.json's verifier_address's key

        // Read the existing bootstrapper config
        let bootstrapper_config_path = "./config/bootstrapper.json";
        let mut config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(bootstrapper_config_path)
                .map_err(|e| SetupError::OtherError(format!("Failed to read bootstrapper config: {}", e)))?
        ).map_err(|e| SetupError::OtherError(format!("Failed to parse bootstrapper config JSON: {}", e)))?;

        // Update the verifier_address field
        config["verifier_address"] = serde_json::Value::String(verifier_address.clone());

        // Write the updated config back to file
        std::fs::write(
            bootstrapper_config_path,
            serde_json::to_string_pretty(&config)
                .map_err(|e| SetupError::OtherError(format!("Failed to serialize config: {}", e)))?
        ).map_err(|e| SetupError::OtherError(format!("Failed to write bootstrapper config: {}", e)))?;

        println!("✅ Updated bootstrapper config with verifier address: {}", verifier_address);

        // TODO: We know this value because anvil creates the same account + pvt key pair on each startup
        // Ideally we would want to ask anvil each time for these values.

        // TODO: I need to know the port from anvil before sending it to bootstrapper!

        println!("🧑‍💻 Starting bootstrapper L1");

        let bootstrapper_l1_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL1)
            .env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
            .env_var("ETH_RPC", "http://localhost:8545")
            .env_var("RUST_LOG", "info")
            .build();

        let status = BootstrapperService::run(bootstrapper_l1_config).await?;
        println!("🥳 Bootstrapper L1 finished with {}", status);

        println!("✅ L1 Setup completed");

        Ok(())
    }

    /// Start L2 setup (Madara, Bootstrapper)
    async fn start_l2_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L2 setup...");

        // Capture values first to avoid borrowing issues
        let madara_port = self.config.madara_port;

        // TODO: Should be validating that dependencies are met (Anvil is running)
        // And is bootstrapped
        // TODO: Should be taking l1 endpoint from anvil!
        let madara_config = MadaraConfigBuilder::new().rpc_port(madara_port).build();

        // Start Madara
        let start_madara = async move {
            let service = MadaraService::start(madara_config).await?;
            println!("✅ Madara started on {}", service.endpoint());
            Ok::<MadaraService, SetupError>(service)
        };

        let madara_service = start_madara.await?;

        // Assign the services
        self.madara = Some(madara_service);

        // // We don't do any transactions on block 0

        let mut is_block_0_mined = false;

        println!("⏳ Waiting for Madara block 0 to be mined");

        // TODO: ideally should be more informed by using madara block time!
        if let Some(madara) = &self.madara {
            while !is_block_0_mined {
                println!("Checking Madara block status...");

                let blk_number = madara.get_latest_block_number().await
                    // TODO: is this the best we can do ?
                    .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
                if blk_number >= 0 {
                    println!("Madara block 0 is mined");
                    is_block_0_mined = true;
                }

                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }

        println!("🧑‍💻 Starting bootstrapper L2");

        // TODO: We know this value because anvil creates the same account + pvt key pair on each startup
        // Ideally we would want to ask anvil each time for these values.

        // TODO: I need to know the port from anvil before sending it to bootstrapper!

        let bootstrapper_l2_config = BootstrapperConfigBuilder::new()
            .mode(BootstrapperMode::SetupL2)
            .config_path(DEFAULT_BOOTSTRAPPER_CONFIG)
            .env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
            .env_var("ETH_RPC", "http://localhost:8545")
            .env_var("RUST_LOG", "info")
            .build();

        let status = BootstrapperService::run(bootstrapper_l2_config).await?;
        println!("🥳 Bootstrapper L2 finished with {}", status);

        println!("✅ L2 Setup completed");

        Ok(())
    }


    /// Start pathfinder and orchestrator service for bootstrapped madara
    async fn start_full_node_syncing(&mut self) ->  Result<(), SetupError> {
        println!("🎯 Starting Pathfinder...");

        // Need to fetch core contract from bootstrapper for orchestrator
        // Need to fetch block number from madara
        // Need to check when pathfinder has been synced till the provided block number
        // Then only orchestrator should start!

        // let mut sync_ready_at_block : u64 = u64::MAX;
        let mut sync_ready_at_block : i64 = 70;

        if let Some(madara) = &self.madara {
            sync_ready_at_block = madara.get_latest_block_number().await
                // TODO: is this the best we can do ?
                .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
            println!("Syncing ready till block number {}", sync_ready_at_block);
        }

        let pathfinder_config = PathfinderConfigBuilder::new()
            .build();

        // Start Pathfinder
        let start_pathfinder = async move {
            let service = PathfinderService::start(pathfinder_config).await?;
            println!("✅ Pathfinder started on {}", service.endpoint());
            Ok::<PathfinderService, SetupError>(service)
        };

        let pathfinder = start_pathfinder.await?;

        // Assign the services
        self.pathfinder = Some(pathfinder);

        println!("Pathfinder has started");

        // A blocking looped logic that checks it pathfinder is ready

        let mut pathfinder_ready = false;

        println!("Waiting for Pathfinder to be ready");

        if let Some(pathfinder_service) = &self.pathfinder {
            while !pathfinder_ready {
                println!("Checking Pathfinder status...");

                let blk_number = pathfinder_service.get_latest_block_number().await
                    // TODO: is this the best we can do ?
                    .map_err(|err| SetupError::Pathfinder(PathfinderError::RpcError(err)))?;
                if blk_number >= sync_ready_at_block {
                    println!("Pathfinder is synced till bootstrapped madara");
                    pathfinder_ready = true;
                }

                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }

        // let pathfinder = PathfinderService::run(pathfinder_config).await?;

        println!("Pathfinder started");
        Ok(())
    }

    /// Start infrastructure services (Anvil, Localstack, MongoDB)
    async fn start_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️ Starting infrastructure services...");

        // // 🔑 KEY: Capture values first to avoid borrowing issues
        let localstack_port = self.config.localstack_port;
        let mongo_port = self.config.mongo_port;

        // Create async closures that DON'T borrow self
        let start_localstack = async move {
            let localstack_config = LocalstackConfigBuilder::new()
                .port(localstack_port)
                .build();

            let service = LocalstackService::start(localstack_config).await?;
            println!("✅ Localstack started on {}", service.endpoint());
            Ok::<LocalstackService, SetupError>(service)
        };


        let start_mongo = async move {
            let mongo_config = MongoConfigBuilder::new()
                .port(mongo_port)
                .build();

            let service = MongoService::start(mongo_config).await?;
            println!("✅ MongoDB started on port {}", service.config().port());
            Ok::<MongoService, SetupError>(service)
        };

        // TODO : need to handle the error from this async move

        // TODO: Atlantic service to be added here later!

        // 🚀 These run in PARALLEL!
        let (mongo_service, localstack_service, ) = tokio::try_join!(start_mongo, start_localstack)?;

        // Assign the services
        self.localstack = Some(localstack_service);
        self.mongo = Some(mongo_service);

        println!("✅ Infrastructure services started");
        Ok(())
    }


    async fn setup_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️ Starting localstack setup ...");

        let orchestrator_setup_config = OrchestratorConfigBuilder::new()
            .mode(OrchestratorMode::Setup)
            .env_var("RUST_LOG", "info")
            .build();

        let _ = OrchestratorService::setup(orchestrator_setup_config).await?;

        Ok(())
    }

    async fn close_services(&mut self) -> Result<(), SetupError> {
        // Anvil should close after L1 setup is completed
        println!("Closing Anvil");
        if let Some(anvil) = self.anvil.take() {
            let _ = anvil.stop();
        }

        Ok(())
    }

    async fn start_mock_prover(&mut self) -> Result<(), SetupError> {
        println!("🔔 Starting Mock Prover Service");

        // Or using the builder:
        let mock_prover_config = MockProverConfigBuilder::new()
            .port(5555)
            .build()?;

        let _mock_prover_service = MockProverService::start(mock_prover_config).await?;

        Ok(())
    }

    // Ideally I need to restart madara before I start orchestration
    // Madara -> from 10 sec to 6 hrs block time
    // Anvil  -> from  1 sec to 6 hrs block time
    // This is so that orchestrator can eventually catch up with madara

    async fn start_orchestration(&mut self) -> Result<(), SetupError> {

        // Start Orchestrator Service, wait for it to complete sync
        println!("🔔 Starting Orchestrator for bootstrapped Madara");

        let orchestrator_setup_config = OrchestratorConfigBuilder::run_l2()
            .port(3000)
            .build();

        let _ = OrchestratorService::start(orchestrator_setup_config).await?;

        Ok(())
    }

    // /// Wait for all services to be ready and responsive
    // async fn wait_for_services_ready(&self) -> Result<(), SetupError> {
    //     println!("⏳ Waiting for services to be ready...");

    //     let mut join_set = JoinSet::new();

    //     // Wait for MongoDB
    //     if let Some(ref mongo) = self.mongo {
    //         join_set.spawn(async {
    //             let mut attempts = 30;
    //             loop {
    //                 if mongo.validate_connection().await.is_ok() {
    //                     break;
    //                 }
    //                 if attempts == 0 {
    //                     return Err(SetupError::Timeout("MongoDB not ready".to_string()));
    //                 }
    //                 attempts -= 1;
    //                 tokio::time::sleep(Duration::from_secs(2)).await;
    //             }
    //             println!("✅ MongoDB is ready");
    //             Ok(())
    //         });
    //     }

    //     // Wait for Localstack
    //     if let Some(ref localstack) = self.localstack {
    //         let aws_prefix = format!("{:?}", self.config.layer).to_lowercase();
    //         join_set.spawn(async move {
    //             let mut attempts = 30;
    //             loop {
    //                 if localstack.validate_resources(&aws_prefix).await.is_ok() {
    //                     break;
    //                 }
    //                 if attempts == 0 {
    //                     return Err(SetupError::Timeout("Localstack not ready".to_string()));
    //                 }
    //                 attempts -= 1;
    //                 tokio::time::sleep(Duration::from_secs(2)).await;
    //             }
    //             println!("✅ Localstack is ready");
    //             Ok(())
    //         });
    //     }

    //     // Wait for Pathfinder (if sync is required)
    //     if self.config.wait_for_sync {
    //         if let Some(ref pathfinder) = self.pathfinder {
    //             join_set.spawn(async {
    //                 let mut attempts = 60; // Longer wait for sync
    //                 loop {
    //                     if pathfinder.validate_connection().await.is_ok() {
    //                         break;
    //                     }
    //                     if attempts == 0 {
    //                         return Err(SetupError::Timeout("Pathfinder not ready".to_string()));
    //                     }
    //                     attempts -= 1;
    //                     tokio::time::sleep(Duration::from_secs(5)).await;
    //                 }
    //                 println!("✅ Pathfinder is ready");
    //                 Ok(())
    //             });
    //         }
    //     }

    //     // Wait for all services
    //     while let Some(result) = join_set.join_next().await {
    //         result.map_err(|e| SetupError::StartupFailed(e.to_string()))??;
    //     }

    //     println!("✅ All services are ready");
    //     Ok(())
    // }

    // /// Run final validation to ensure setup is complete
    // async fn run_setup_validation(&self) -> Result<(), SetupError> {
    //     println!("🔍 Running final validation...");

    //     // Validate that all endpoints are responsive
    //     let endpoints = vec![
    //         &self.context.anvil_endpoint,
    //         &self.context.localstack_endpoint,
    //         &self.context.pathfinder_endpoint,
    //         &self.context.sequencer_endpoint,
    //         &self.context.bootstrapper_endpoint,
    //     ];

    //     for endpoint in endpoints {
    //         // Basic connectivity check (you might want more sophisticated validation)
    //         let url = url::Url::parse(endpoint)
    //             .map_err(|e| SetupError::ContextFailed(format!("Invalid endpoint {}: {}", endpoint, e)))?;

    //         if let Ok(addr) = format!("{}:{}", url.host_str().unwrap_or("127.0.0.1"), url.port().unwrap_or(80))
    //             .parse::<std::net::SocketAddr>()
    //         {
    //             match tokio::net::TcpStream::connect(addr).await {
    //                 Ok(_) => println!("✅ {} is responsive", endpoint),
    //                 Err(_) => return Err(SetupError::StartupFailed(format!("Endpoint {} not responsive", endpoint))),
    //             }
    //         }
    //     }

    //     println!("✅ All validations passed");
    //     Ok(())
    // }

    // /// Stop all services gracefully
    // pub async fn stop_all(&mut self) -> Result<(), SetupError> {
    //     println!("🛑 Stopping all services...");

    //     // Stop in reverse order of startup
    //     if let Some(ref mut bootstrapper) = self.bootstrapper {
    //         bootstrapper.stop()?;
    //         println!("🛑 Bootstrapper stopped");
    //     }

    //     if let Some(ref mut sequencer) = self.sequencer {
    //         sequencer.stop()?;
    //         println!("🛑 Sequencer stopped");
    //     }

    //     if let Some(ref mut orchestrator) = self.orchestrator {
    //         orchestrator.stop()?;
    //         println!("🛑 Orchestrator stopped");
    //     }

    //     if let Some(ref mut pathfinder) = self.pathfinder {
    //         pathfinder.stop()?;
    //         println!("🛑 Pathfinder stopped");
    //     }

    //     if let Some(ref mut mongo) = self.mongo {
    //         mongo.stop()?;
    //         println!("🛑 MongoDB stopped");
    //     }

    //     if let Some(ref mut localstack) = self.localstack {
    //         localstack.stop()?;
    //         println!("🛑 Localstack stopped");
    //     }

    //     if let Some(ref mut anvil) = self.anvil {
    //         anvil.stop()?;
    //         println!("🛑 Anvil stopped");
    //     }

    //     println!("✅ All services stopped");
    //     Ok(())
    // }

    // /// Get the current context
    // pub fn context(&self) -> Arc<Context> {
    //     Arc::clone(&self.context)
    // }

    // /// Check if setup is complete and all services are running
    // pub fn is_ready(&self) -> bool {
    //     self.anvil.is_some()
    //         && self.localstack.is_some()
    //         && self.mongo.is_some()
    //         && self.pathfinder.is_some()
    //         && self.orchestrator.is_some()
    //         && self.sequencer.is_some()
    //         && self.bootstrapper.is_some()
    // }

    /// Get setup configuration
    pub fn config(&self) -> &SetupConfig {
        &self.config
    }
}

// impl Drop for Setup {
//     fn drop(&mut self) {
//         // Attempt graceful shutdown on drop
//         let rt = tokio::runtime::Runtime::new();
//         if let Ok(rt) = rt {
//             let _ = rt.block_on(self.stop_all());
//         }
//     }
// }

// // Helper functions for creating common setups
// impl Setup {
//     /// Create a quick L2 development setup
//     pub async fn quick_l2_dev(ethereum_api_key: String) -> Result<Self, SetupError> {
//         let config = SetupConfig {
//             layer: Layer::L2,
//             ethereum_api_key,
//             wait_for_sync: false,                    // Skip sync for faster dev setup
//             setup_timeout: Duration::from_secs(180), // 3 minutes
//             ..Default::default()
//         };
//         Self::l2_setup(config).await
//     }

//     /// Create a full L2 production setup
//     pub async fn full_l2_production(ethereum_api_key: String, data_dir: String) -> Result<Self, SetupError> {
//         let config = SetupConfig {
//             layer: Layer::L2,
//             ethereum_api_key,
//             data_directory: data_dir,
//             wait_for_sync: true,
//             setup_timeout: Duration::from_secs(600), // 10 minutes
//             ..Default::default()
//         };
//         Self::l2_setup(config).await
//     }

//     /// Create a quick L3 development setup
//     pub async fn quick_l3_dev(ethereum_api_key: String) -> Result<Self, SetupError> {
//         let config = SetupConfig {
//             layer: Layer::L3,
//             ethereum_api_key,
//             wait_for_sync: false,
//             setup_timeout: Duration::from_secs(180),
//             ..Default::default()
//         };
//         Self::l3_setup(config).await
//     }
//
//  /// Start core services (Pathfinder, Orchestrator, Sequencer, Bootstrapper)
// async fn start_core_services(&mut self) -> Result<(), SetupError> {
//     println!("🎯 Starting core services...");

//     // 🔑 KEY: Capture values first to avoid borrowing issues
//     let anvil_port = self.config.anvil_port;
//     let pathfinder_port = self.config.pathfinder_port;
//     let data_directory = self.config.data_directory.clone();
//     let madara_port = self.config.madara_port;

//     // Create async closures that DON'T borrow self
//     let start_anvil = async move {
//         let anvil_config = AnvilConfigBuilder::new()
//             .port(anvil_port)
//             .build();

//         let service = AnvilService::start(anvil_config).await?;
//         println!("✅ Anvil started on {}", service.server().endpoint());
//         Ok::<AnvilService, SetupError>(service)
//     };

//     // Start Madara
//     let start_madara = async move {
//         let madara_config = MadaraConfigBuilder::new()
//             .with_rpc_port(madara_port)
//             .build();

//         let service = MadaraService::start(madara_config).await?;
//         println!("✅ Madara started on {}", service.endpoint());
//         Ok::<MadaraService, SetupError>(service)
//     };

//     // // Pathfinder should start only after madara is ready!
//     // let start_pathfinder = async move {
//     //     let mut pathfinder_config = PathfinderConfig::default();
//     //     pathfinder_config.port = pathfinder_port;
//     //     pathfinder_config.data_volume = Some(format!("{}/pathfinder", data_directory));

//     //     let service = PathfinderService::start(pathfinder_config).await?;
//     //     println!("✅ Pathfinder started on {}", service.endpoint());
//     //     Ok::<PathfinderService, SetupError>(service)
//     // };

//     // 🚀 These run in PARALLEL!
//     let (anvil_service, madara_service) = tokio::try_join!(start_anvil, start_madara)?;

//     // Assign the services
//     self.anvil = Some(anvil_service);
//     self.madara = Some(madara_service);
//     // self.pathfinder = Some(pathfinder_service);

//     sleep(Duration::from_secs(100)).await;

//     println!("✅ Core services started");
//     Ok(())
// }

// }
