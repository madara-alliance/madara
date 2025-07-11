use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::{sleep, timeout};

// Import all the services we've created
use crate::servers::anvil::{AnvilCMDBuilder, AnvilConfig, AnvilError, AnvilService};
use crate::servers::bootstrapper::{BootstrapperConfigBuilder, BootstrapperConfig, BootstrapperError, BootstrapperMode, BootstrapperService};
use crate::servers::docker::{DockerError, DockerServer};
use crate::servers::bootstrapper::DEFAULT_BOOTSTRAPPER_CONFIG;
use crate::servers::localstack::{LocalstackConfig, LocalstackError, LocalstackService};
use crate::servers::madara::{MadaraCMDBuilder, MadaraConfig, MadaraError, MadaraService};
use crate::servers::mongo::{MongoConfig, MongoError, MongoService};
use crate::constants::{DEFAULT_DATA_DIR};
use crate::servers::orchestrator::{
    Layer, OrchestratorConfig, OrchestratorError, OrchestratorMode, OrchestratorService,
};
use crate::servers::pathfinder::{PathfinderConfig, PathfinderError, PathfinderService};
use std::collections::HashMap;

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
    #[error("Setup timeout: {0}")]
    Timeout(String),
    #[error("Dependency validation failed: {0}")]
    DependencyFailed(String),
    #[error("Service startup failed: {0}")]
    StartupFailed(String),
    #[error("Context initialization failed: {0}")]
    ContextFailed(String),
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

enum Services {
    Anvil(AnvilService),
    Localstack(LocalstackService),
    Mongo(MongoService),
    Pathfinder(PathfinderService),
}

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
            println!("Chain state exists, starting servers...");
            // self.start_existing_chain().await?;
        } else {
            println!("Chain state does not exist, setting up new chain...");
            self.setup_new_chain().await?;
        }

        // Wrap the entire setup in a timeout
        timeout(self.config.setup_timeout, async {
            self.validate_dependencies().await?;
            // self.check_existing_databases().await?;
            // self.start_infrastructure_services().await?;
            // self.wait_for_services_ready().await?;
            // self.run_setup_validation().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup process timed out".to_string()))??;

        // Timeout this for 5 mins
        timeout(Duration::from_secs(300), async {
            self.start_l1_setup().await?;
            // self.wait_for_services_ready().await?;
            // self.run_setup_validation().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L1 process timed out".to_string()))??;

        // Timeout this for 30 mins
        timeout(Duration::from_secs(1800), async {
            self.start_l2_setup().await?;
            // self.wait_for_services_ready().await?;
            // self.run_setup_validation().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L2 process timed out".to_string()))??;

        println!("✅ Setup completed successfully in {:?}", self.context.elapsed());
        Ok(())
    }

    /// Validate all required dependencies
    async fn validate_dependencies(&self) -> Result<(), SetupError> {
        println!("🔍 Validating dependencies...");

        let mut join_set = JoinSet::new();

        // Validate Docker
        join_set.spawn(async {
            if !DockerServer::is_docker_running() {
                println!("Docker is NOT running");
                return Err(SetupError::DependencyFailed("Docker not running".to_string()));
            }
            println!("Docker is running");
            Ok(())
        });

        // Validate Anvil
        join_set.spawn(async {
            let result = std::process::Command::new("anvil").arg("--version").output();
            if result.is_err() {
                return Err(SetupError::DependencyFailed("Anvil not found".to_string()));
            }
            println!("Anvil is available");
            Ok(())
        });

        // Wait for all validations
        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        println!("✅ All dependencies validated");
        Ok(())
    }

    /// Check if existing databases need to be preserved or cleared
    async fn check_existing_chain_state(&self) -> Result<bool, SetupError> {
        println!("🗄️  Checking existing databases...");

        if !self.config.skip_existing_dbs {
            // Create data directory if it doesn't exist
            tokio::fs::create_dir_all(&self.config.data_directory)
                .await
                .map_err(|e| SetupError::ContextFailed(format!("Failed to create data directory: {}", e)))?;

            println!("📁 Data directory prepared: {}", self.config.data_directory);
        } else {
            println!("⏭️  Skipping database initialization (existing DBs will be used)");
        }

        Ok(true)
    }


    /// Starts afresh chain
    async fn setup_new_chain(&mut self) -> Result<(), SetupError> {

        // Step 1: Setup L1
        // Spin up Anvil
        // Use bootstrapper to setup L1
        // Timeout this for 5 mins

        // TODO: take these timeouts from config
        timeout(Duration::from_secs(300), async {
            self.start_l1_setup().await?;
            // self.wait_for_services_ready().await?;
            // self.run_setup_validation().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L1 process timed out".to_string()))??;

        // Step 2: Setup L2
        // Spin up Anvil
        // Use bootstrapper to setup L2

        // TODO: take these timeouts from config
        timeout(Duration::from_secs(1800), async {
            self.start_l2_setup().await?;
            // self.wait_for_services_ready().await?;
            // self.run_setup_validation().await?;
            Ok::<(), SetupError>(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Setup L1 process timed out".to_string()))??;





        Ok(())


    }










    /// Start infrastructure services (Anvil, Localstack, MongoDB)
    async fn start_infrastructure_services(&mut self) -> Result<(), SetupError> {
        println!("🏗️  Starting infrastructure services...");

        // 🔑 KEY: Capture values first to avoid borrowing issues
        let localstack_port = self.config.localstack_port;
        let layer = self.config.layer.clone();
        let mongo_port = self.config.mongo_port;

        // Create async closures that DON'T borrow self
        let start_localstack = async move {
            let localstack_config = LocalstackConfig {
                port: localstack_port,
                aws_prefix: Some(format!("{:?}", layer).to_lowercase()),
                ..Default::default()
            };

            let service = LocalstackService::start(localstack_config).await?;
            println!("✅ Localstack started on {}", service.server().endpoint());
            Ok::<LocalstackService, SetupError>(service)
        };

        let start_mongo = async move {
            let mongo_config = MongoConfig { port: mongo_port, ..Default::default() };

            let service = MongoService::start(mongo_config).await?;
            println!("✅ MongoDB started on port {}", service.server().port());
            Ok::<MongoService, SetupError>(service)
        };

        // TODO: Atlantic get's added here later!

        // 🚀 These run in PARALLEL!
        let (localstack_service, mongo_service) = tokio::try_join!(start_localstack, start_mongo)?;

        // Assign the services
        self.localstack = Some(localstack_service);
        self.mongo = Some(mongo_service);

        println!("✅ Infrastructure services started");
        Ok(())
    }

    /// Start core services (Pathfinder, Orchestrator, Sequencer, Bootstrapper)
    async fn start_core_services(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting core services...");

        // 🔑 KEY: Capture values first to avoid borrowing issues
        let anvil_port = self.config.anvil_port;
        let pathfinder_port = self.config.pathfinder_port;
        let data_directory = self.config.data_directory.clone();
        let madara_port = self.config.madara_port;

        // Create async closures that DON'T borrow self
        let start_anvil = async move {
            let anvil_config = AnvilConfig { port: anvil_port, ..Default::default() };

            let service = AnvilService::start(anvil_config).await?;
            println!("✅ Anvil started on {}", service.server().endpoint());
            Ok::<AnvilService, SetupError>(service)
        };

        // Start Madara
        let start_madara = async move {
            let mut madara_config = MadaraConfig::default();
            madara_config.rpc_port = madara_port;

            let service = MadaraService::start(madara_config).await?;
            println!("✅ Madara started on {}", service.endpoint());
            Ok::<MadaraService, SetupError>(service)
        };

        // // Pathfinder should start only after madara is ready!
        // let start_pathfinder = async move {
        //     let mut pathfinder_config = PathfinderConfig::default();
        //     pathfinder_config.port = pathfinder_port;
        //     pathfinder_config.data_volume = Some(format!("{}/pathfinder", data_directory));

        //     let service = PathfinderService::start(pathfinder_config).await?;
        //     println!("✅ Pathfinder started on {}", service.endpoint());
        //     Ok::<PathfinderService, SetupError>(service)
        // };

        // 🚀 These run in PARALLEL!
        let (anvil_service, madara_service) = tokio::try_join!(start_anvil, start_madara)?;

        // Assign the services
        self.anvil = Some(anvil_service);
        self.madara = Some(madara_service);
        // self.pathfinder = Some(pathfinder_service);

        sleep(Duration::from_secs(100)).await;

        println!("✅ Core services started");
        Ok(())
    }

    /// Start L1 setup (Anvil, Bootstrapper)
    async fn start_l1_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L1 setup...");

        // No need to do load state on setup!
        // Only dump state on shutdown!

        // Anvil db path :
        let anvil_db_path = format!("{}/anvil.json", self.config.db_dir_path.clone());

        let anvil_config = AnvilCMDBuilder::new().port(self.config.anvil_port.clone()).dump_state(anvil_db_path).build();

        // Create async closures that DON'T borrow self
        let start_anvil = async move {
            let service = AnvilService::start(anvil_config).await?;
            println!("✅ Anvil started on {}", service.server().endpoint());
            Ok::<AnvilService, SetupError>(service)
        };
        let anvil_service = start_anvil.await?;


        // Assign the services
        self.anvil = Some(anvil_service);

        println!("Anvil has started");

        println!("Starting bootstrapper L1");

        // TODO: We know this value because anvil creates the same account + pvt key pair on each startup
        // Ideally we would want to ask anvil each time for these values.

        // TODO: I need to know the port from anvil before sending it to bootstrapper!

        let bootstrapper_l1_config = BootstrapperConfigBuilder::new()
            .with_mode(BootstrapperMode::SetupL1)
            .add_env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
            .add_env_var("ETH_RPC", "http://localhost:8545")
            .add_env_var("RUST_LOG", "info")
            .build();

        let bootstrapper = BootstrapperService::new(bootstrapper_l1_config)?;

        let bootstrapper_l1_setup = bootstrapper.run()
            .await
            .map_err(|err| SetupError::Bootstrapper(err))?;

        println!("L1 Setup completed");

        Ok(())
    }

    /// Start L2 setup (Madara, Bootstrapper)
    async fn start_l2_setup(&mut self) -> Result<(), SetupError> {
        println!("🎯 Starting L2 setup...");

        // Capture values first to avoid borrowing issues
        let madara_port = self.config.madara_port;

        // TODO: Should be validating that dependencies are met (Anvil is running)
        // TODO: Should be taking l1 endpoint from anvil!
        let madara_config = MadaraCMDBuilder::new().with_rpc_port(madara_port).build();

        // Start Madara
        let start_madara = async move {
            let service = MadaraService::start(madara_config).await?;
            println!("✅ Madara started on {}", service.endpoint());
            Ok::<MadaraService, SetupError>(service)
        };

        let madara_service = start_madara.await?;

        // Assign the services
        self.madara = Some(madara_service);

        println!("Madara has started");

        println!("Starting bootstrapper L2");

        // TODO: We know this value because anvil creates the same account + pvt key pair on each startup
        // Ideally we would want to ask anvil each time for these values.

        // TODO: I need to know the port from anvil before sending it to bootstrapper!

        let bootstrapper_l2_config = BootstrapperConfigBuilder::new()
            .with_mode(BootstrapperMode::SetupL2)
            .with_config_path(DEFAULT_BOOTSTRAPPER_CONFIG)
            .add_env_var("ETH_PRIVATE_KEY", "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80")
            .add_env_var("ETH_RPC", "http://localhost:8545")
            .add_env_var("RUST_LOG", "info")
            .build();

        let bootstrapper = BootstrapperService::new(bootstrapper_l2_config)?;

        let bootstrapper_l2_setup = bootstrapper.run()
            .await
            .map_err(|err| SetupError::Bootstrapper(err))?;

        println!("L2 Setup completed");
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
// }
