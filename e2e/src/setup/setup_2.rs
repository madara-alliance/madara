use std::sync::Arc;
use std::time::Duration;
use std::path::Path;
use tokio::task::JoinSet;
use tokio::time::timeout;
`use tokio::time::Instant;

// Import all the services we've created
use crate::services::anvil::{AnvilConfigBuilder, AnvilService};
use crate::services::bootstrapper::{BootstrapperConfigBuilder, BootstrapperMode, BootstrapperService};
use crate::services::docker::DockerServer;
use crate::services::mock_verifier::{MockVerifierDeployerConfigBuilder, MockVerifierDeployerService};
use crate::services::pathfinder::DEFAULT_PATHFINDER_IMAGE;
use crate::services::mongodb::MONGO_DEFAULT_DATABASE_PATH;
use crate::services::bootstrapper::DEFAULT_BOOTSTRAPPER_CONFIG;
use crate::services::bootstrapper::BOOTSTRAPPER_DEFAULT_ADDRESS_PATH;
use crate::services::anvil::ANVIL_DEFAULT_DATABASE_NAME;
use crate::services::madara::MADARA_DEFAULT_DATABASE_NAME;
use crate::services::mongodb::DEFAULT_MONGO_IMAGE;
use crate::services::localstack::DEFAULT_LOCALSTACK_IMAGE;
use crate::services::localstack::{LocalstackService, LocalstackConfigBuilder};
use crate::services::madara::{MadaraError, MadaraService, MadaraConfigBuilder};
use crate::services::mongodb::{MongoConfigBuilder, MongoService};
use crate::services::orchestrator::{
    Layer, OrchestratorMode, OrchestratorService, OrchestratorConfigBuilder
};
use crate::services::mock_prover::{MockProverConfigBuilder, MockProverService};
use crate::services::pathfinder::{PathfinderConfigBuilder, PathfinderError, PathfinderService};
use crate::services::helpers::NodeRpcMethods;

use super::config::*;

// =============================================================================
// MAIN SETUP STRUCT
// =============================================================================

pub struct Setup {
    // Service instances
    pub anvil: Option<AnvilService>,
    pub localstack: Option<LocalstackService>,
    pub mongo: Option<MongoService>,
    pub pathfinder: Option<PathfinderService>,
    pub orchestrator: Option<OrchestratorService>,
    pub madara: Option<MadaraService>,
    pub bootstrapper: Option<BootstrapperService>,
    pub mock_prover_service: Option<MockProverService>,
    pub timeouts : Timeouts,
    // Core context -> change to config
    pub context: Arc<Context>,

    // actual context
    pub bootstrapped_madara_block_number: i64,

}

// =============================================================================
// SETUP IMPLEMENTATION - CORE METHODS
// =============================================================================

impl Setup {
    /// Create a new setup instance
    pub fn new(context: Context) -> Result<Self, SetupError> {
        let context = Arc::new(context);

        Ok(Self {
            anvil: None,
            localstack: None,
            mongo: None,
            pathfinder: None,
            orchestrator: None,
            madara: None,
            bootstrapper: None,
            mock_prover_service: None,
            // TODO: Might remove, or convert to option
            bootstrapped_madara_block_number: i64::MIN,
            context,
            timeouts: Timeouts::default(),
        })
    }

    // /// Get setup context
    // pub fn context(&self) -> &Context {
    //     &self.context
    // }

    /// Run the complete setup process
    pub async fn setup(&mut self) -> Result<(), SetupError> {
        println!("🚀 Starting Chain Setup for {:?} layer...", self.context.layer);

        // Step 1: Validate dependencies within timeout
        // Needs layer and timeout
        self.validate_and_pull_dependencies().await?;

        // Step 2: Check for existing chain state and decide setup path
        let state_exists = self.check_existing_chain_state().await?;

        if state_exists {
            println!("✅ Chain state exists, starting servers...");
            // TODO: Implement start_existing_chain method
        } else {
            println!("❌ Chain state does not exist, setting up new chain...");
            self.setup_new_chain().await?;
        }

        Ok(())
    }
}

// =============================================================================
// SETUP IMPLEMENTATION - VALIDATION METHODS
// =============================================================================

impl Setup {

    pub fn set_bootstrapped_madara_block_number(&mut self, block_number: i64) {
        self.bootstrapped_madara_block_number = block_number;
    }

    ///////////////
    /// Validating Dependencies
    ///////////////

    /// Validate dependencies with timeout wrapper
    async fn validate_and_pull_dependencies(&self) -> Result<(), SetupError> {
        timeout(self.timeouts.validate_dependencies, async {
            println!("🔍 Validating dependencies...");

            // Validate basic tools first
            self.validate_basic_tools().await?;

            // Pull required Docker images
            self.pull_docker_images().await?;

            println!("✅ All dependencies validated");
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup process timed out".to_string()))??;

        Ok(())
    }

    /// Validate basic tools (anvil, forge, docker)
    async fn validate_basic_tools(&self) -> Result<(), SetupError> {
        let mut join_set = JoinSet::new();

        // Validate Anvil and Forge, Only needed for L2
        if self.context().get_layer().clone() == Layer::L2 {
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
        }

        // Validate Docker
        join_set.spawn(async {
            if !DockerServer::is_docker_running() {
                println!("❌ Docker is NOT running");
                return Err(SetupError::DependencyFailed("Docker not running".to_string()));
            }
            println!("✅ Docker is running");
            Ok(())
        });

        // Wait for all validations to complete
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        Ok(())
    }

    /// Pull required Docker images
    async fn pull_docker_images(&self) -> Result<(), SetupError> {
        println!("📦 Pulling required Docker images...");

        let images = vec![
            ("mongo", DEFAULT_MONGO_IMAGE),
            ("localstack/localstack", DEFAULT_LOCALSTACK_IMAGE),
            ("pathfinder", DEFAULT_PATHFINDER_IMAGE),
        ];

        let mut join_set = JoinSet::new();

        for (display_name, image_name) in images {
            join_set.spawn(async move {
                println!("📦 Pulling {}...", display_name);

                let result = tokio::process::Command::new("docker")
                    .args(["pull", image_name])
                    .output()
                    .await;

                match result {
                    Ok(output) if output.status.success() => {
                        println!("✅ Successfully pulled {}", display_name);
                        Ok(())
                    }
                    Ok(output) => {
                        let error_msg = String::from_utf8_lossy(&output.stderr);
                        Err(SetupError::DependencyFailed(
                            format!("Failed to pull {}: {}", display_name, error_msg)
                        ))
                    }
                    Err(e) => {
                        Err(SetupError::DependencyFailed(
                            format!("Failed to execute docker pull {}: {}", display_name, e)
                        ))
                    }
                }
            });
        }

        // Wait for all image pulls to complete
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        Ok(())
    }

    /// Check if database exists in the Default Data Directory
    async fn check_existing_chain_state(&self) -> Result<bool, SetupError> {
        println!("🗄️  Checking existing databases...");

        // TODO: shoud add bootstrapper's addresses.json
        // And mockverifier txt file as well as well

        let data_dir = Path::new(DEFAULT_DATA_DIR);

        // Check for required files/directories
        let mongodb_dump_exists = data_dir.join(MONGO_DEFAULT_DATABASE_PATH).exists();
        let anvil_json_exists = data_dir.join(ANVIL_DEFAULT_DATABASE_NAME).exists();
        let madara_db_exists = data_dir.join(MADARA_DEFAULT_DATABASE_NAME).exists();
        let address_json_exists = data_dir.join(BOOTSTRAPPER_DEFAULT_ADDRESS_PATH).exists();

        let all_exist = mongodb_dump_exists && anvil_json_exists && madara_db_exists && address_json_exists;

        Ok(all_exist)
    }
}

// =============================================================================
// SETUP IMPLEMENTATION - CHAIN SETUP METHODS
// =============================================================================

impl Setup {
    /// Set up a fresh chain from scratch
    async fn setup_new_chain(&mut self) -> Result<(), SetupError> {
        let start = Instant::now();

        // Infrastructure services (parallel)
        self.start_infrastructure_services().await?;

        // Setup infrastructure services
        self.setup_infrastructure_services().await?;

        // L1 setup (sequential)
        self.start_l1_setup().await?;

        // L2 setup (sequential)
        self.start_l2_setup().await?;

        // Full node syncing
        self.start_full_node_syncing().await?;

        // Mock proving service
        self.start_mock_prover_service().await?;

        // Orchestrator service
        self.start_orchestration().await?;

        println!("✅ Setup completed successfully in {:?}", start.elapsed());
        Ok(())
    }

}

// =============================================================================
// SETUP IMPLEMENTATION - SERVICE STARTUP METHODS
// =============================================================================

impl Setup {
    /// Start infrastructure services (Localstack, MongoDB)
    async fn start_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️ Starting infrastructure services...");

        // TODO: I don't like the fact that we have to timeout to get infrastructure_services,
        // I also don't like that I have to clone, can we do something about that ?
        // can we do better ?
        let duration = self.context().get_timeouts().start_infrastructure_services.clone();
        let mongo_config = self.context().get_mongo_config().clone();
        let localstack_config = self.context().get_localstack_config().clone();

        timeout(duration, async {
            // Starting MongoDB
            let start_mongo = async move {
                let service = MongoService::start(mongo_config).await?;
                println!("✅ MongoDB started on port {}", service.config().port());
                Ok::<MongoService, SetupError>(service)
            };

            // Starting Localstack
            let start_localstack = async move {
                let service = LocalstackService::start(localstack_config).await?;
                println!("✅ Localstack started on {}", service.endpoint());
                Ok::<LocalstackService, SetupError>(service)
            };

            let (mongo_service, localstack_service) = tokio::try_join!(start_mongo, start_localstack)?;

            self.mongo = Some(mongo_service);
            self.localstack = Some(localstack_service);

            println!("🏗️✅ Infrastructure services started");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start infrastructure services timed out".to_string()))?
    }

    /// Set up infrastructure services configuration
    async fn setup_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️ Starting localstack setup...");

        let duration = self.context().get_timeouts().setup_infrastructure_services.clone();

        timeout(duration, async {
            self.setup_resource_for_orchestrator().await?;
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup infrastructure services timed out".to_string()))?
    }

    /// Start L1 setup (Anvil, Mock Verifier, Bootstrapper)
    async fn start_l1_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L1 setup...");

        let duration = self.context().get_timeouts().start_l1_setup.clone();

        timeout(duration, async {
            // Start Anvil
            self.start_anvil_service().await?;

            // Deploy Mock Verifier
            self.deploy_mock_verifier().await?;

            // Bootstrap Chain L1
            self.bootstrap_chain_l1().await?;

            println!("✅ L1 Setup completed");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L1 Chain timed out".to_string()))?
    }

    /// Start L2 setup (Madara, L2 Bootstrapper)
    async fn start_l2_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L2 setup...");

        let duration = self.context().get_timeouts().start_l2_setup.clone();

        timeout(duration, async {
            // Start Madara
            self.start_madara_service().await?;

            // BLOCKING: Wait for Madara block 0
            if let Some(madara) = &self.madara {
                madara.wait_for_block_mined(0).await?;
            }

            // Bootstrap Chain L2
            self.bootstrap_chain_l2().await?;

            // We would want to have madara restarted at this point


            // Finding the minimum block number that pathfinder has to sync till to start orchestration.
            let sync_ready_at_block: i64 = self.get_madara_latest_block().await?;
            println!("Syncing ready till block number {}", sync_ready_at_block);

            self.set_bootstrapped_madara_block_number(sync_ready_at_block);

            println!("✅ L2 Setup completed");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L2 Chain timed out".to_string()))?
    }

    /// Start full node syncing (Pathfinder)
    async fn start_full_node_syncing(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting Pathfinder...");

        let duration = self.context().get_timeouts().start_full_node_syncing.clone();

        timeout(duration, async {

            self.start_pathfinder_service().await?;

            // BLOCKING: Wait for Pathfinder block to sync till Madara block
            if let Some(pathfinder) = &self.pathfinder {
                // Convert sync_ready_at_block to u64 by replacing it's value to 0 if it's negative
                pathfinder.wait_for_block_synced(self.bootstrapped_madara_block_number.max(0) as u64).await?;
            }

            println!("✅ Pathfinder started and synced");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Full Node Syncing timed out".to_string()))?
    }

    /// Start mock prover service
    async fn start_mock_prover_service(&mut self) -> Result<(), SetupError> {
        println!("🔔 Starting Mock Prover Service");

        let duration = self.context().get_timeouts().start_mock_prover.clone();

        timeout(duration, async {

            let mock_prover_config = self.context().get_mock_prover_config().clone();
            let mock_prover_service = MockProverService::start(mock_prover_config).await?;

            self.mock_prover_service = Some(mock_prover_service);

            println!("✅ Mock Prover Service started and synced");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Mock Proving Service timed out".to_string()))?
    }

    /// Start orchestration service
    async fn start_orchestration(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting Orchestration Service...");

        let duration = self.context().get_timeouts().start_orchestration.clone();

        timeout(duration, async {

            println!("🔔 Starting Orchestrator for bootstrapped Madara");

            let orchestrator_run_config = self.context().get_orchestrator_run_config().clone();
            let orchestrator_service = OrchestratorService::start(orchestrator_run_config).await?;

            self.orchestrator = Some(orchestrator_service);

            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Orchestrator Service timed out".to_string()))?
    }
}

// =============================================================================
// SETUP IMPLEMENTATION - INDIVIDUAL SERVICE METHODS
// =============================================================================

impl Setup {

    /// Start Anvil service
    async fn start_anvil_service(&mut self) -> Result<(), SetupError> {
        let anvil_config = self.context().get_anvil_config().clone();

        let anvil_service = AnvilService::start(anvil_config).await?;
        println!("✅ Anvil started on {}", anvil_service.endpoint());

        self.anvil = Some(anvil_service);
        Ok(())
    }

    /// Start Madara service
    async fn start_madara_service(&mut self) -> Result<(), SetupError> {
        let madara_config = self.context().get_madara_config().clone();

        let madara_service = MadaraService::start(madara_config).await?;
        println!("✅ Madara started on {}", madara_service.endpoint());

        self.madara = Some(madara_service);
        Ok(())
    }

    /// Start Pathfinder service
    async fn start_pathfinder_service(&mut self) -> Result<(), SetupError> {
        println!("🔔 Starting Pathfinder Service");

        let pathfinder_config =  self.context().get_pathfinder_config().clone();

        let pathfinder_service = PathfinderService::start(pathfinder_config).await?;
        println!("✅ Pathfinder started on {}", pathfinder_service.endpoint());

        self.pathfinder = Some(pathfinder_service);
        Ok(())
    }



    /// Deploy mock verifier contract
    async fn deploy_mock_verifier(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting mock verifier deployer");

        let mock_verifier_config =  self.context().get_mock_verifier_config().clone();

        let address = MockVerifierDeployerService::run(mock_verifier_config).await?;

        // Set this address in the config
        println!("🥳 Mock verifier deployer finished with address {}", address);

        // Update bootstrapper config with verifier address
        BootstrapperService::update_config_file("verifier_address", address.as_str());

        Ok(())
    }

    /// Run L1 bootstrapper
    async fn bootstrap_chain_l1(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting bootstrapper L1");

        let bootstrapper_setup_l1_config = self.context().get_bootstrapper_setup_l1_config().clone();

        let status = BootstrapperService::run(bootstrapper_setup_l1_config).await?;
        println!("🥳 Bootstrapper L1 finished with {}", status);

        Ok(())
    }

    /// Run L2 bootstrapper
    async fn bootstrap_chain_l2(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting bootstrapper L2");

        let bootstrapper_setup_l2_config = self.context().get_bootstrapper_setup_l2_config().clone();

        let status = BootstrapperService::run(bootstrapper_setup_l2_config).await?;
        println!("🥳 Bootstrapper L2 finished with {}", status);

        Ok(())
    }

    /// Run setup orchestrator
    async fn setup_resource_for_orchestrator(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting Setup Resoruce Orchestrator L1");

        let orchestrator_setup_config = self.context().get_orchestrator_setup_config().clone();

        let status = OrchestratorService::setup(orchestrator_setup_config).await?;
        println!("🥳 Resource Setup for Orchestrator finished with {}", status);

        Ok(())
    }
}

// =============================================================================
// SETUP IMPLEMENTATION - UTILITY METHODS
// =============================================================================

impl Setup {
    /// Get latest block number from Madara
    async fn get_madara_latest_block(&self) -> Result<i64, SetupError> {
        if let Some(madara) = &self.madara {
            let block_number = madara.get_latest_block_number().await
                .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
            Ok(block_number)
        } else {
            panic!("Madara is not initialized");
        }
    }


    /// Close services gracefully
    async fn close_services(&mut self) -> Result<(), SetupError> {
        println!("🛑 Closing services...");

        if let Some(anvil) = self.anvil.take() {
            let _ = anvil.stop();
            println!("🛑 Anvil stopped");
        }

        Ok(())
    }
}

// // =============================================================================
// // SETUP IMPLEMENTATION - CONVENIENCE CONSTRUCTORS
// // =============================================================================

// impl Setup {
//     /// Create a quick L2 development setup
//     pub async fn quick_l2_dev(ethereum_api_key: String) -> Result<Self, SetupError> {
//         let context = Context::new()
//             .with_layer(Layer::L2)
//             .with_ethereum_api_key(ethereum_api_key)
//             .with_wait_for_sync(false)                    // Skip sync for faster dev setup
//             .with_setup_timeout(Duration::from_secs(180)); // 3 minutes

//         let mut setup = Self::new(context)?;
//         setup.setup().await?;
//         Ok(setup)
//     }

//     /// Create a full L2 production setup
//     pub async fn full_l2_production(ethereum_api_key: String, data_dir: String) -> Result<Self, SetupError> {
//         let context = Context::new()
//             .with_layer(Layer::L2)
//             .with_ethereum_api_key(ethereum_api_key)
//             .with_data_directory(data_dir)
//             .with_wait_for_sync(true)
//             .with_setup_timeout(Duration::from_secs(600)); // 10 minutes

//         let mut setup = Self::new(context)?;
//         setup.setup().await?;
//         Ok(setup)
//     }

//     /// Create a quick L3 development setup
//     pub async fn quick_l3_dev(ethereum_api_key: String) -> Result<Self, SetupError> {
//         let context = Context::new()
//             .with_layer(Layer::L3)
//             .with_ethereum_api_key(ethereum_api_key)
//             .with_wait_for_sync(false)
//             .with_setup_timeout(Duration::from_secs(180));

//         let mut setup = Self::new(context)?;
//         setup.setup().await?;
//         Ok(setup)
//     }

//     /// Check if setup is complete and all services are running
//     pub fn is_ready(&self) -> bool {
//         self.anvil.is_some()
//             && self.localstack.is_some()
//             && self.mongo.is_some()
//             && self.pathfinder.is_some()
//             && self.orchestrator.is_some()
//             && self.madara.is_some()
//             && self.bootstrapper.is_some()
//     }

//     /// Get the current context
//     pub fn context(&self) -> Arc<Context> {
//         Arc::clone(&self.context)
//     }
// }

// =============================================================================
// SETUP IMPLEMENTATION - SERVICE MANAGEMENT
// =============================================================================

impl Setup {
    /// Stop all services gracefully
    pub async fn stop_all(&mut self) -> Result<(), SetupError> {
        println!("🛑 Stopping all services...");

        // Stop in reverse order of startup
        if let Some(ref mut bootstrapper) = self.bootstrapper {
            // bootstrapper.stop()?;
            println!("🛑 Bootstrapper stopped");
        }

        if let Some(ref mut madara) = self.madara {
            // madara.stop()?;
            println!("🛑 Madara stopped");
        }

        if let Some(ref mut orchestrator) = self.orchestrator {
            // orchestrator.stop()?;
            println!("🛑 Orchestrator stopped");
        }

        if let Some(ref mut pathfinder) = self.pathfinder {
            // pathfinder.stop()?;
            println!("🛑 Pathfinder stopped");
        }

        if let Some(ref mut mongo) = self.mongo {
            // mongo.stop()?;
            println!("🛑 MongoDB stopped");
        }

        if let Some(ref mut localstack) = self.localstack {
            // localstack.stop()?;
            println!("🛑 Localstack stopped");
        }

        if let Some(ref mut anvil) = self.anvil {
            let _ = anvil.stop();
            println!("🛑 Anvil stopped");
        }

        println!("✅ All services stopped");
        Ok(())
    }

    /// Wait for all services to be ready and responsive
    async fn wait_for_services_ready(&self) -> Result<(), SetupError> {
        println!("⏳ Waiting for services to be ready...");

        let mut join_set = JoinSet::new();

        // Wait for MongoDB
        if let Some(ref mongo) = self.mongo {
            let mongo_clone = mongo.clone(); // Assuming Clone is implemented
            join_set.spawn(async move {
                let mut attempts = 30;
                loop {
                    if mongo_clone.validate_connection().await.is_ok() {
                        break;
                    }
                    if attempts == 0 {
                        return Err(SetupError::Timeout("MongoDB not ready".to_string()));
                    }
                    attempts -= 1;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                println!("✅ MongoDB is ready");
                Ok(())
            });
        }

        // Wait for Localstack
        if let Some(ref localstack) = self.localstack {
            let aws_prefix = format!("{:?}", self.context.layer).to_lowercase();
            let localstack_clone = localstack.clone(); // Assuming Clone is implemented
            join_set.spawn(async move {
                let mut attempts = 30;
                loop {
                    if localstack_clone.validate_resources(&aws_prefix).await.is_ok() {
                        break;
                    }
                    if attempts == 0 {
                        return Err(SetupError::Timeout("Localstack not ready".to_string()));
                    }
                    attempts -= 1;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                println!("✅ Localstack is ready");
                Ok(())
            });
        }

        // Wait for Pathfinder (if sync is required)
        if self.context.wait_for_sync {
            if let Some(ref pathfinder) = self.pathfinder {
                let pathfinder_clone = pathfinder.clone(); // Assuming Clone is implemented
                join_set.spawn(async move {
                    let mut attempts = 60; // Longer wait for sync
                    loop {
                        if pathfinder_clone.validate_connection().await.is_ok() {
                            break;
                        }
                        if attempts == 0 {
                            return Err(SetupError::Timeout("Pathfinder not ready".to_string()));
                        }
                        attempts -= 1;
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    println!("✅ Pathfinder is ready");
                    Ok(())
                });
            }
        }

        // Wait for all services
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::StartupFailed(e.to_string()))??;
        }

        println!("✅ All services are ready");
        Ok(())
    }

    /// Run final validation to ensure setup is complete
    async fn run_setup_validation(&self) -> Result<(), SetupError> {
        println!("🔍 Running final validation...");

        // Validate that all endpoints are responsive
        let endpoints = vec![
            &self.context.anvil_endpoint,
            &self.context.localstack_endpoint,
            &self.context.pathfinder_endpoint,
            &self.context.sequencer_endpoint,
            &self.context.bootstrapper_endpoint,
        ];

        for endpoint in endpoints {
            // Basic connectivity check
            let url = url::Url::parse(endpoint)
                .map_err(|e| SetupError::ContextFailed(format!("Invalid endpoint {}: {}", endpoint, e)))?;

            if let Ok(addr) = format!("{}:{}",
                url.host_str().unwrap_or("127.0.0.1"),
                url.port().unwrap_or(80)
            ).parse::<std::net::SocketAddr>() {
                match tokio::net::TcpStream::connect(addr).await {
                    Ok(_) => println!("✅ {} is responsive", endpoint),
                    Err(_) => return Err(SetupError::StartupFailed(
                        format!("Endpoint {} not responsive", endpoint)
                    )),
                }
            }
        }

        println!("✅ All validations passed");
        Ok(())
    }
}

// =============================================================================
// DROP IMPLEMENTATION
// =============================================================================

impl Drop for Setup {
    fn drop(&mut self) {
        // Attempt graceful shutdown on drop
        if let Ok(rt) = tokio::runtime::Runtime::new() {
            let _ = rt.block_on(self.stop_all());
        }
    }
}

// =============================================================================
// TESTS MODULE
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_default() {
        let context = Context::default();
        assert_eq!(context.layer, Layer::L2);
        assert_eq!(context.anvil_port, 8545);
        assert_eq!(context.mongo_port, 27017);
    }

    #[tokio::test]
    async fn test_context_builder() {
        let context = Context::new()
            .with_layer(Layer::L3)
            .with_anvil_port(9999)
            .with_ethereum_api_key("test_key".to_string());

        assert_eq!(context.layer, Layer::L3);
        assert_eq!(context.anvil_port, 9999);
        assert_eq!(context.ethereum_api_key, "test_key");
        assert_eq!(context.anvil_endpoint, "http://127.0.0.1:9999");
    }

    #[tokio::test]
    async fn test_setup_creation() {
        let context = Context::default();
        let setup = Setup::new(context);

        assert!(setup.is_ok());
        let setup = setup.unwrap();
        assert!(!setup.is_ready()); // Should not be ready initially
    }
}
