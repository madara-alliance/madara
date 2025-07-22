// =============================================================================
// SETUP
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;
use std::sync::Arc;
use std::path::Path;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tokio::time::Instant;
use tokio::process::Command;
use tokio::time::Duration;
use tokio::fs;
use fs_extra::dir::{copy, CopyOptions};
// Import all the services we've created
use crate::services::anvil::AnvilService;
use crate::services::bootstrapper::BootstrapperService;
use crate::services::docker::DockerServer;
use crate::services::mock_verifier::MockVerifierDeployerService;
use crate::services::constants::*;
use crate::services::mock_verifier::DEFAULT_VERIFIER_FILE_NAME;
use crate::services::pathfinder::DEFAULT_PATHFINDER_IMAGE;
use crate::services::bootstrapper::BOOTSTRAPPER_DEFAULT_ADDRESS_PATH;
use crate::services::anvil::ANVIL_DEFAULT_DATABASE_NAME;
use crate::services::madara::MADARA_DEFAULT_DATABASE_NAME;
use crate::services::mongodb::DEFAULT_MONGO_IMAGE;
use crate::services::localstack::DEFAULT_LOCALSTACK_IMAGE;
use crate::services::localstack::LocalstackService;
use crate::services::madara::{MadaraService, MadaraError};
use crate::services::mongodb::MongoService;
use crate::services::orchestrator::{
    Layer, OrchestratorService
};
use crate::services::mock_prover::MockProverService;
use crate::services::pathfinder::PathfinderService;
use crate::services::helpers::NodeRpcMethods;

#[derive(Debug, PartialEq, serde::Serialize)]
pub enum DBState {
    /// Database is ready to copy
    ReadyToUse,
    /// Database is being created, hence locked
    Locked,
    /// Database is not ready to copy, can be created
    NotReady,
    /// Database is in error state
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
// MAIN SETUP STRUCT
// =============================================================================

pub struct Setup {
    pub config: Arc<SetupConfig>,

    // context
    pub bootstrapped_madara_block_number: i64,

    // Service instances
    pub anvil_service: Option<AnvilService>,
    pub localstack_service: Option<LocalstackService>,
    pub mongo_service: Option<MongoService>,
    pub pathfinder_service: Option<PathfinderService>,
    pub orchestrator_service: Option<OrchestratorService>,
    pub madara_service: Option<MadaraService>,
    pub bootstrapper_service: Option<BootstrapperService>,
    pub mock_prover_service: Option<MockProverService>,
    pub timeouts : Timeouts,
}

// =============================================================================
// SETUP IMPLEMENTATION - CORE METHODS
// =============================================================================

impl Setup {
    /// Create a new setup instance
    pub fn new(config: SetupConfig) -> Result<Self, SetupError> {
        let config = Arc::new(config);

        Ok(Self {
            anvil_service: None,
            localstack_service: None,
            mongo_service: None,
            pathfinder_service: None,
            orchestrator_service: None,
            madara_service: None,
            bootstrapper_service: None,
            mock_prover_service: None,
            // TODO: Might remove, or convert to option
            bootstrapped_madara_block_number: i64::MIN,
            config,
            timeouts: Timeouts::default(),
        })
    }

    /// Get setup context
    pub fn config(&self) -> &SetupConfig {
        &self.config
    }

    /// Run the complete setup process
    pub async fn setup(&mut self, test_name: &str) -> Result<(), SetupError> {
        println!("🚀 Starting Chain Setup for {:?} layer...", self.config.layer);

        // Step 1: Check for existing chain state and decide setup path
        let db_status = self.check_existing_chain_state().await?;

        if db_status == DBState::ReadyToUse {
            println!("✅ Chain state exists, starting servers...");
        }
        else if db_status == DBState::Locked {
            println!("⚠️ Chain state is locked, waiting for unlock...");
            // TODO: Implement timeout and re-call setup
            unimplemented!("Implement timeout and re-call setup");
        }
        else if db_status == DBState::NotReady {
            println!("❌ Chain state does not exist, setting up new chain...");
            self.setup_new_chain().await?;
            println!("✅ Chain state created, using...");
        }
        else {
            return Err(SetupError::OtherError(format!("DB Status invalid")));
        }

        self.start_chain(test_name).await?;
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
        let duration = self.config().get_timeouts().validate_dependencies;

        let result = timeout(duration, async {
            println!("🔍 Validating dependencies...");

            // Validate basic tools first - this should fail fast
            self.validate_basic_tools().await?;

            // Pull required Docker images
            self.pull_docker_images().await?;

            println!("✅ All dependencies validated");
            Ok::<(), SetupError>(())
        }).await;

        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                Err(e)
            }, // Inner function failed - this should happen immediately
            Err(_) => Err(SetupError::Timeout("Setup process timed out".to_string())),
        }
    }

    /// Validate basic tools (anvil, forge, docker)
    async fn validate_basic_tools(&self) -> Result<(), SetupError> {
        let mut join_set = JoinSet::new();

        // Validate Anvil and Forge, Only needed for L2
        if self.config().get_layer().clone() == Layer::L2 {
            // Anvil
            join_set.spawn(async {
                let result = Command::new("anvil").arg("--version").output().await;
                if result.is_err() {
                    return Err(SetupError::DependencyFailed("❌ Anvil not found".to_string()));
                }
                println!("✅ Anvil is available");
                Ok(())
            });

            // Forge
            join_set.spawn(async {
                let result = Command::new("forge").arg("--version").output().await;
                if result.is_err() {
                    return Err(SetupError::DependencyFailed("❌ Forge not found".to_string()));
                }
                println!("✅ Forge is available");
                Ok(())
            });
        }

        // Validate Docker
        join_set.spawn(async {
            if !DockerServer::is_docker_running().await {
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

                let result = Command::new("docker")
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
    async fn check_existing_chain_state(&self) -> Result<DBState, SetupError> {
        println!("🗄️ Checking existing databases...");

        let data_dir = Path::new(DEFAULT_DATA_DIR);

        // Read the status off Status file
        let status_file = data_dir.join("STATUS");
        let status = match fs::read_to_string(&status_file).await {
            Ok(s) => s.into(),
            // if file is not found, return not ready
            Err(_) => DBState::NotReady,
        };

        println!("Pre Existing DB Status: {:?}", status);

        if status == DBState::ReadyToUse {
            // Extra validation checks
            // Check for required files/directories
            // TODO: fix : let mongodb_dump_exists = data_dir.join(MONGO_DEFAULT_DATABASE_PATH).exists();
            let anvil_json_exists = data_dir.join(ANVIL_DEFAULT_DATABASE_NAME).exists();
            let madara_db_exists = data_dir.join(MADARA_DEFAULT_DATABASE_NAME).exists();
            let address_json_exists = data_dir.join(BOOTSTRAPPER_DEFAULT_ADDRESS_PATH).exists();
            let mock_verifier_exists = data_dir.join(DEFAULT_VERIFIER_FILE_NAME).exists();

            let all_exist =
                anvil_json_exists && madara_db_exists &&
                address_json_exists && mock_verifier_exists;

            if all_exist {
                Ok(DBState::ReadyToUse)
            } else {
                Err(SetupError::OtherError("Database not ready, but status is ReadyToUse".to_string()))
            }
        } else {
            Ok(status)
        }

    }
}

// =============================================================================
// SETUP IMPLEMENTATION - CHAIN SETUP METHODS
// =============================================================================

impl Setup {
    /// Set up a fresh chain from scratch
    async fn setup_new_chain(&mut self) -> Result<(), SetupError> {
        let start = Instant::now();

        // Validate dependencies within timeout
        self.validate_and_pull_dependencies().await?;

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

        if let Err(cleanup_err) = self.stop_all().await {
            println!("Failed to cleanup: {}", cleanup_err);
        }

        // TODO: create a file in data saying status : ReadyToUse
        // shutdown all the services
        println!("✅ Setup completed successfully in {:?}", start.elapsed());
        Ok(())
    }


    async fn start_chain(&mut self, test_name: &str) -> Result<(), SetupError> {
        let start = Instant::now();

        // Copy the database
        self.copy_databases(test_name).await?;

        // Use the newly created data_test_name directory for db of all services

        // // Infrastructure services (parallel)
        // self.start_infrastructure_services().await?;

        // // Setup infrastructure services
        // self.setup_infrastructure_services().await?;

        // // Start Anvil, Madara, Pathfinder
        // // Mock Prover, Orchestrator services (parallel)
        // self.start_anvil_service().await?;
        // self.start_madara_service().await?;
        // self.start_pathfinder_service().await?;
        // self.start_mock_prover_service().await?;
        // // self.start_orchestration(db_dir).await?;

        // shutdown all the services
        println!("✅ Starting Chain completed successfully in {:?}", start.elapsed());
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
        let duration = self.config().get_timeouts().start_infrastructure_services.clone();
        let mongo_config = self.config().get_mongo_config().clone();
        let localstack_config = self.config().get_localstack_config().clone();

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

            self.mongo_service = Some(mongo_service);
            self.localstack_service = Some(localstack_service);

            println!("🏗️✅ Infrastructure services started");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start infrastructure services timed out".to_string()))?
    }

    /// Set up infrastructure services configuration
    async fn setup_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️ Starting localstack setup...");

        let duration = self.config().get_timeouts().setup_infrastructure_services.clone();

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

        let duration = self.config().get_timeouts().start_l1_setup.clone();

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

        let duration = self.config().get_timeouts().start_l2_setup.clone();

        timeout(duration, async {
            // Start Madara
            self.start_madara_service().await?;

            // BLOCKING: Wait for Madara block 0
            if let Some(madara) = &self.madara_service {
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

        let duration = self.config().get_timeouts().start_full_node_syncing.clone();

        timeout(duration, async {

            self.start_pathfinder_service().await?;

            // BLOCKING: Wait for Pathfinder block to sync till Madara block
            if let Some(pathfinder) = &self.pathfinder_service {
                // Convert sync_ready_at_block to u64 by replacing it's value to 0 if it's negative
                pathfinder.wait_for_block_synced(self.bootstrapped_madara_block_number.max(0) as u64).await?;
            }

            println!("✅ Pathfinder started and synced");

            if let Some(mut madara) = self.madara_service.take() {
                madara.stop()?;
                println!("🛑 Madara stopped, after Pathfinder synced");

            }

            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Start Full Node Syncing timed out".to_string()))?
    }

    /// Start mock prover service
    async fn start_mock_prover_service(&mut self) -> Result<(), SetupError> {
        println!("🔔 Starting Mock Prover Service");

        let duration = self.config().get_timeouts().start_mock_prover.clone();

        timeout(duration, async {

            let mock_prover_config = self.config().get_mock_prover_config().clone();
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

        let duration = self.config().get_timeouts().start_orchestration.clone();

        timeout(duration, async {

            println!("🔔 Starting Orchestrator for bootstrapped Madara");

            let orchestrator_run_config = self.config().get_orchestrator_run_config().clone();
            let orchestrator_service = OrchestratorService::run(orchestrator_run_config).await?;

            self.orchestrator_service = Some(orchestrator_service);

            let max_retries = 20;
            let delay = Duration::from_secs(360);

            let mut retries = 0;
            loop {
                println!("⏳ Checking orchestrator state update...");
                let block_number = self.check_orchestrator_state_update(self.bootstrapped_madara_block_number.max(0) as u64).await?;
                if block_number {
                    break;
                }
                if retries >= max_retries {
                    return Err(SetupError::Timeout("Orchestrator state update till bootstrapped madara block timed out".to_string()));
                }
                println!("😩 Retrying orchestrator state update check...");
                retries += 1;
                tokio::time::sleep(delay).await;
            }

            // Ideally: TODO: Restart orchestrator while dumping the database !
            // So that no jobs are inn LockedForProcessing while the database is being dumped
            if let Some(ref mut  mongo) = self.mongo_service {
                println!("Dumping Mongodb database...");
                if let Some(ref orchestrator) = self.orchestrator_service.as_mut() {
                    mongo.dump_db(orchestrator.config().database_name()).await?;
                }
            }

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
        let anvil_config = self.config().get_anvil_config().clone();

        let anvil_service = AnvilService::start(anvil_config).await?;
        println!("✅ Anvil started on {}", anvil_service.endpoint());

        self.anvil_service = Some(anvil_service);
        Ok(())
    }

    /// Start Madara service
    async fn start_madara_service(&mut self) -> Result<(), SetupError> {
        let madara_config = self.config().get_madara_config().clone();

        let madara_service = MadaraService::start(madara_config).await?;
        println!("✅ Madara started on {}", madara_service.endpoint());

        self.madara_service = Some(madara_service);
        Ok(())
    }

    /// Start Pathfinder service
    async fn start_pathfinder_service(&mut self) -> Result<(), SetupError> {
        println!("🔔 Starting Pathfinder Service");

        let pathfinder_config =  self.config().get_pathfinder_config().clone();

        let pathfinder_service = PathfinderService::start(pathfinder_config).await?;
        println!("✅ Pathfinder started on {}", pathfinder_service.endpoint());

        self.pathfinder_service = Some(pathfinder_service);
        Ok(())
    }


    /// Deploy mock verifier contract
    async fn deploy_mock_verifier(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting mock verifier deployer");

        let mock_verifier_config =  self.config().get_mock_verifier_deployer_config().clone();

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

        let bootstrapper_setup_l1_config = self.config().get_bootstrapper_setup_l1_config().clone();

        let status = BootstrapperService::run(bootstrapper_setup_l1_config).await?;
        println!("🥳 Bootstrapper L1 finished with {}", status);

        Ok(())
    }

    /// Run L2 bootstrapper
    async fn bootstrap_chain_l2(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting bootstrapper L2");

        let bootstrapper_setup_l2_config = self.config().get_bootstrapper_setup_l2_config().clone();

        let status = BootstrapperService::run(bootstrapper_setup_l2_config).await?;
        println!("🥳 Bootstrapper L2 finished with {}", status);

        Ok(())
    }

    /// Run setup orchestrator
    async fn setup_resource_for_orchestrator(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Starting Setup Resoruce Orchestrator L1");

        let orchestrator_setup_config = self.config().get_orchestrator_setup_config().clone();

        let status = OrchestratorService::setup(orchestrator_setup_config).await?;
        println!("🥳 Resource Setup for Orchestrator finished with {}", status);

        Ok(())
    }
}

// =============================================================================
// SETUP IMPLEMENTATION - UTILITY METHODS
// =============================================================================

impl Setup {

    /// Copy the databases to a new directory
    async fn copy_databases(&self, dir_suffix: &str) -> Result<(), SetupError> {
        let data_test_name = format!("data_{}", dir_suffix);
        println!("🧑‍💻 Copying databases to {}", data_test_name);

        let mut options = CopyOptions::new();
        options.overwrite = true;
        options.copy_inside = true;

        copy(DEFAULT_DATA_DIR, &data_test_name, &options)
            .map_err(|e| SetupError::OtherError(e.to_string()))?;

        Ok(())
    }

    /// Get latest block number from Madara
    async fn get_madara_latest_block(&self) -> Result<i64, SetupError> {
        if let Some(madara) = &self.madara_service {
            let block_number = madara.get_latest_block_number().await
                .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
            Ok(block_number)
        } else {
            panic!("Madara is not initialized");
        }
    }

    /// Check if Orchestrator completed state update for specified block
    async fn check_orchestrator_state_update(&self, block_number: u64) -> Result<bool, SetupError> {
        if let Some(orchestrator) = &self.orchestrator_service {
            let status = orchestrator.check_state_update(block_number).await
                .map_err(|err| SetupError::Orchestrator(err))?;
            Ok(status)
        } else {
            panic!("Orchestrator is not initialized");
        }
    }

}

// =============================================================================
// SETUP IMPLEMENTATION - SERVICE MANAGEMENT
// =============================================================================

impl Setup {
    /// Stop all services gracefully
    pub async fn stop_all(&mut self) -> Result<(), SetupError> {
        println!("🛑 Stopping all services...");

        // Stop in reverse order of startup
        if let Some(ref mut bootstrapper) = self.bootstrapper_service {
            let _ = bootstrapper.stop()?;
            println!("🛑 Bootstrapper stopped");
        }

        if let Some(ref mut madara) = self.madara_service {
            let _ = madara.stop()?;
            println!("🛑 Madara stopped");
        }

        if let Some(ref mut orchestrator) = self.orchestrator_service {
            let _ = orchestrator.stop()?;
            println!("🛑 Orchestrator stopped");
        }

        if let Some(ref mut pathfinder) = self.pathfinder_service {
            let _ = pathfinder.stop()?;
            println!("🛑 Pathfinder stopped");
        }

        if let Some(ref mut mongo) = self.mongo_service {
            let _ = mongo.stop()?;
            println!("🛑 MongoDB stopped");
        }

        if let Some(ref mut localstack) = self.localstack_service {
            let _ = localstack.stop()?;
            println!("🛑 Localstack stopped");
        }

        if let Some(ref mut anvil) = self.anvil_service {
            let _ = anvil.stop();
            println!("🛑 Anvil stopped");
        }

        println!("✅ All services stopped");
        Ok(())
    }

}

// =============================================================================
// DROP IMPLEMENTATION
// =============================================================================

impl Drop for Setup {
    fn drop(&mut self) {
        // This attempts to use the existing runtime if available, otherwise creates a new one
        // Try to use the current runtime first
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            // We're inside a runtime, spawn a detached task
            handle.spawn(async move {
                // Note: self is moved here, but Drop is consuming anyway
                // You'll need to adjust this based on your stop_all implementation
                // let _ = self.stop_all().await;
            });
        } else {
            // No runtime exists, safe to create one
            if let Ok(rt) = tokio::runtime::Runtime::new() {
                let _ = rt.block_on(self.stop_all());
            }
        }
    }
}
