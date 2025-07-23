// =============================================================================
// SERVICE MANAGEMENT
// =============================================================================

use std::sync::Arc;

// Import all the services we've created
use crate::services::anvil::AnvilService;
use crate::services::bootstrapper::BootstrapperService;
use crate::services::mock_verifier::MockVerifierDeployerService;

use crate::services::localstack::LocalstackService;
use crate::services::madara::{MadaraService, MadaraError};
use crate::services::mongodb::MongoService;
use crate::services::orchestrator::OrchestratorService;
pub use super::config::*;
use crate::services::orchestrator::DEFAULT_ORCHESTRATOR_DATABASE_NAME;
use crate::services::mock_prover::MockProverService;
use crate::services::pathfinder::PathfinderService;
use crate::services::helpers::NodeRpcMethods;
use tokio::time::sleep;

use tokio::time::{timeout, Instant, Duration};

pub struct ServiceManager {
    config: Arc<SetupConfig>,
    bootstrapped_madara_block_number: i64,
}

#[derive(Default)]
pub struct RunningServices {
    pub anvil_service: Option<AnvilService>,
    pub localstack_service: Option<LocalstackService>,
    pub mongo_service: Option<MongoService>,
    pub pathfinder_service: Option<PathfinderService>,
    pub orchestrator_service: Option<OrchestratorService>,
    pub madara_service: Option<MadaraService>,
    pub mock_prover_service: Option<MockProverService>,
}

impl ServiceManager {
    pub fn new(config: Arc<SetupConfig>) -> Self {
        Self {
            config,
            bootstrapped_madara_block_number: i64::MIN,
        }
    }

    pub async fn setup_new_chain(&mut self) -> Result<(), SetupError> {
        let start = Instant::now();

        let mut services = RunningServices::default();

        // Infrastructure first
        self.start_infrastructure(&mut services).await?;
        self.setup_localstack_infrastructure().await?;

        // L1 setup
        self.setup_l1_chain(&mut services).await?;

        // L2 setup
        self.setup_l2_chain(&mut services).await?;

        // Full node syncing
        self.setup_full_node_syncing(&mut services).await?;

        // Mock proving
        self.setup_mock_prover(&mut services).await?;

        // Orchestration
        self.setup_orchestration(&mut services).await?;

        // Cleanup setup services
        self.cleanup_setup_services(&mut services).await?;

        println!("✅ Setup completed successfully in {:?}", start.elapsed());
        Ok(())
    }

    pub async fn start_runtime_services(&self) -> Result<RunningServices, SetupError> {
        let mut services = RunningServices::default();

        // Start infrastructure
        self.start_infrastructure(&mut services).await?;
        self.setup_localstack_infrastructure().await?;
        self.restore_mongodb_database(&services).await?;

        // Start runtime services
        self.start_anvil(&mut services).await?;
        self.start_madara(&mut services).await?;
        self.start_pathfinder(&mut services).await?;
        self.start_mock_prover(&mut services).await?;
        self.start_orchestrator(&mut services).await?;

        Ok(services)
    }

    async fn start_infrastructure(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🏗️ Starting infrastructure services...");

        let duration = self.config.get_timeouts().start_infrastructure_services;

        timeout(duration, async {
            let mongo_config = self.config.get_mongo_config().clone();
            let localstack_config = self.config.get_localstack_config().clone();

            let start_mongo = async {
                let service = MongoService::start(mongo_config).await?;
                println!("✅ MongoDB started on port {}", service.config().port());
                Ok::<MongoService, SetupError>(service)
            };

            let start_localstack = async {
                let service = LocalstackService::start(localstack_config).await?;
                println!("✅ Localstack started on {}", service.endpoint());
                Ok::<LocalstackService, SetupError>(service)
            };

            let (mongo_service, localstack_service) = tokio::try_join!(start_mongo, start_localstack)?;

            services.mongo_service = Some(mongo_service);
            services.localstack_service = Some(localstack_service);

            println!("🏗️✅ Infrastructure services started");
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("Infrastructure startup timed out".to_string()))?
    }

    async fn setup_localstack_infrastructure(&self) -> Result<(), SetupError> {
        println!("🏗️ Setting up localstack infrastructure...");

        let duration = self.config.get_timeouts().setup_localstack_infrastructure_services;

        timeout(duration, async {
            let orchestrator_setup_config = self.config.get_orchestrator_setup_config().clone();

            let status = OrchestratorService::setup(orchestrator_setup_config).await?;
            println!("🥳 Resource Setup for Orchestrator finished with {}", status);
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("LocalstackInfrastructure setup timed out".to_string()))?
    }

    async fn restore_mongodb_database(&self, services: &RunningServices) -> Result<(), SetupError> {
        println!("🏗️ Setting up mongodb infrastructure...");

        let duration = self.config.get_timeouts().setup_mongodb_infrastructure_services;

        timeout(duration, async {
            if let Some(ref mongo) = services.mongo_service {
                mongo.restore_db(DEFAULT_ORCHESTRATOR_DATABASE_NAME).await?;
            }
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("Mongodb Infrastructure setup timed out".to_string()))?
    }

    async fn setup_l1_chain(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting L1 setup...");

        let duration = self.config.get_timeouts().start_l1_setup;

        timeout(duration, async {
            self.start_anvil(services).await?;
            self.deploy_mock_verifier().await?;
            self.bootstrap_l1().await?;
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("L1 setup timed out".to_string()))?
    }

    async fn setup_l2_chain(&mut self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting L2 setup...");

        let duration = self.config.get_timeouts().start_l2_setup;

        timeout(duration, async {
            self.start_madara(services).await?;

            if let Some(madara) = &services.madara_service {
                madara.wait_for_block_mined(0).await?;
            }

            self.bootstrap_l2().await?;

            // Get the block number for syncing
            if let Some(madara) = &services.madara_service {
                self.bootstrapped_madara_block_number = madara.get_latest_block_number().await
                    .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
            }

            println!("✅ L2 Setup completed");
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("L2 setup timed out".to_string()))?
    }

    async fn setup_full_node_syncing(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting Pathfinder syncing...");

        let duration = self.config.get_timeouts().start_full_node_syncing;

        timeout(duration, async {
            self.start_pathfinder(services).await?;

            if let Some(pathfinder) = &services.pathfinder_service {
                let sync_block = self.bootstrapped_madara_block_number.max(0) as u64;
                pathfinder.wait_for_block_synced(sync_block).await?;
            }

            // Stop Madara after Pathfinder syncs
            if let Some(mut madara) = services.madara_service.take() {
                madara.stop()?;
                println!("🛑 Madara stopped after Pathfinder sync");
            }

            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("Pathfinder syncing timed out".to_string()))?
    }

    async fn setup_mock_prover(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        self.start_mock_prover(services).await
    }

    async fn setup_orchestration(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting Orchestration...");

        let duration = self.config.get_timeouts().start_orchestration;

        timeout(duration, async {
            self.start_orchestrator(services).await?;

            self.wait_for_orchestrator_sync(services).await?;
            self.dump_databases(services).await?;

            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("Orchestration setup timed out".to_string()))?
    }

    async fn wait_for_orchestrator_sync(&self, services: &RunningServices) -> Result<(), SetupError> {
        let max_retries = 20;
        let delay = Duration::from_secs(360);

        for retry in 0..max_retries {
            println!("⏳ Checking orchestrator state update... (attempt {})", retry + 1);

            if let Some(orchestrator) = &services.orchestrator_service {
                let sync_block = self.bootstrapped_madara_block_number.max(0) as u64;
                let is_synced = orchestrator.check_state_update(sync_block).await
                    .map_err(|err| SetupError::Orchestrator(err))?;

                if is_synced {
                    return Ok(());
                }
            }

            if retry < max_retries - 1 {
                println!("😩 Retrying orchestrator state update check...");
                tokio::time::sleep(delay).await;
            }
        }

        Err(SetupError::Timeout("Orchestrator sync timed out".to_string()))
    }

    async fn dump_databases(&self, services: &RunningServices) -> Result<(), SetupError> {
        if let (Some(mongo), Some(orchestrator)) = (&services.mongo_service, &services.orchestrator_service) {
            println!("Dumping MongoDB database...");
            mongo.dump_db(orchestrator.config().database_name()).await?;
        }
        Ok(())
    }

    async fn cleanup_setup_services(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        // Stop all services used during setup
        if let Some(mut madara) = services.madara_service.take() {
            madara.stop()?;
        }
        if let Some(mut orchestrator) = services.orchestrator_service.take() {
            orchestrator.stop()?;
        }
        if let Some(mut pathfinder) = services.pathfinder_service.take() {
            pathfinder.stop()?;
        }
        if let Some(mut mock_prover) = services.mock_prover_service.take() {
            mock_prover.stop()?;
        }
        if let Some(mut mongo) = services.mongo_service.take() {
            mongo.stop()?;
        }
        if let Some(mut localstack) = services.localstack_service.take() {
            localstack.stop()?;
        }
        if let Some(mut anvil) = services.anvil_service.take() {
            anvil.stop()?;
        }

        // Docker takes a while to close the containers
        sleep(Duration::from_secs(10)).await;
        Ok(())
    }

    // Individual service startup methods
    async fn start_anvil(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        let anvil_config = self.config.get_anvil_config().clone();
        let anvil_service = AnvilService::start(anvil_config).await?;
        println!("✅ Anvil started on {}", anvil_service.endpoint());
        services.anvil_service = Some(anvil_service);
        Ok(())
    }

    async fn start_madara(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        let madara_config = self.config.get_madara_config().clone();
        let madara_service = MadaraService::start(madara_config).await?;
        println!("✅ Madara started on {}", madara_service.endpoint());
        services.madara_service = Some(madara_service);
        Ok(())
    }

    async fn start_pathfinder(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        let pathfinder_config = self.config.get_pathfinder_config().clone();
        let pathfinder_service = PathfinderService::start(pathfinder_config).await?;
        println!("✅ Pathfinder started on {}", pathfinder_service.endpoint());
        services.pathfinder_service = Some(pathfinder_service);
        Ok(())
    }

    async fn start_mock_prover(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🔔 Starting Mock Prover Service");

        let duration = self.config.get_timeouts().start_mock_prover;

        timeout(duration, async {
            let mock_prover_config = self.config.get_mock_prover_config().clone();
            let mock_prover_service = MockProverService::start(mock_prover_config).await?;
            services.mock_prover_service = Some(mock_prover_service);
            println!("✅ Mock Prover Service started");
            Ok(())
        }).await
        .map_err(|_| SetupError::Timeout("Mock Prover startup timed out".to_string()))?
    }

    // Deployment and bootstrap methods
    async fn deploy_mock_verifier(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Deploying mock verifier...");

        let mock_verifier_config = self.config.get_mock_verifier_deployer_config().clone();
        let address = MockVerifierDeployerService::run(mock_verifier_config).await?;

        println!("🥳 Mock verifier deployed at address {}", address);
        let _ =BootstrapperService::update_config_file("verifier_address", address.as_str());

        Ok(())
    }

    async fn bootstrap_l1(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Bootstrapping L1...");

        let bootstrapper_config = self.config.get_bootstrapper_setup_l1_config().clone();
        let status = BootstrapperService::run(bootstrapper_config).await?;

        println!("🥳 L1 Bootstrapper finished with {}", status);
        Ok(())
    }

    async fn bootstrap_l2(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Bootstrapping L2...");

        let bootstrapper_config = self.config.get_bootstrapper_setup_l2_config().clone();
        let status = BootstrapperService::run(bootstrapper_config).await?;

        println!("🥳 L2 Bootstrapper finished with {}", status);
        Ok(())
    }

    async fn start_orchestrator(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        let orchestrator_config = self.config.get_orchestrator_run_config().clone();
        let orchestrator_service = OrchestratorService::run(orchestrator_config).await?;
        services.orchestrator_service = Some(orchestrator_service);
        Ok(())
    }

}
