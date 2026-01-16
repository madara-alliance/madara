// =============================================================================
// SERVICE MANAGEMENT
// =============================================================================

use std::sync::Arc;

// Import all the services we've created
use crate::services::anvil::AnvilService;
use crate::services::bootstrapper_v2::{BootstrapperV2Error, BootstrapperV2Service};
use crate::services::mock_verifier::MockVerifierDeployerService;

pub use super::config::*;
use crate::services::constants::*;
use crate::services::localstack::LocalstackService;
use crate::services::madara::{MadaraError, MadaraService};
use crate::services::mongodb::MongoService;
use crate::services::orchestrator::{OrchestratorConfig, OrchestratorError, OrchestratorService};

use crate::services::helpers::{retry_with_timeout, NodeRpcMethods};
use crate::services::mock_prover::MockProverService;
use crate::services::pathfinder::PathfinderService;
use tokio::time::{timeout, Duration, Instant};

pub struct ServiceManager {
    config: Arc<SetupConfig>,
    bootstrapped_madara_block_number: Option<u64>,
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
        Self { config, bootstrapped_madara_block_number: None }
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
        // self.setup_mock_prover(&mut services).await?;

        // Orchestration
        self.setup_orchestration(&mut services).await?;

        println!("✅ Setup completed successfully in {:?}", start.elapsed());
        Ok(())
    }

    pub async fn start_runtime_services(&self) -> Result<RunningServices, SetupError> {
        let mut services = RunningServices::default();

        // Start infrastructure
        self.start_infrastructure(&mut services).await?;
        self.setup_localstack_infrastructure().await?;
        self.restore_mongodb_database(&services).await?;

        // // Start runtime services
        self.start_anvil(&mut services).await?;
        self.start_madara(&mut services).await?;
        self.start_pathfinder(&mut services).await?;
        // self.start_mock_prover(&mut services).await?;
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
        })
        .await
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
        })
        .await
        .map_err(|_| SetupError::Timeout("LocalstackInfrastructure setup timed out".to_string()))?
    }

    async fn setup_l1_chain(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting L1 setup...");

        let duration = self.config.get_timeouts().complete_l1_setup;

        timeout(duration, async {
            self.start_anvil(services).await?;

            // Update bootstrapper v2 config with the dynamic Anvil RPC URL
            if let Some(anvil) = &services.anvil_service {
                let anvil_endpoint = anvil.endpoint();
                BootstrapperV2Service::update_config_file("base_layer.rpc_url", anvil_endpoint.as_str())?;
                println!("✅ Updated bootstrapper v2 config with base_layer.rpc_url: {}", anvil_endpoint);
            }

            self.deploy_mock_verifier().await?;
            self.bootstrap_base().await?;
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("L1 setup timed out".to_string()))?
    }

    async fn setup_l2_chain(&mut self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting L2 setup...");

        let duration = self.config.get_timeouts().complete_l2_setup;

        timeout(duration, async {
            self.start_madara(services).await?;
            // sleep(Duration::from_secs(12)).await;
            if let Some(madara) = &services.madara_service {
                madara.wait_for_block_mined(0).await?;

                // Update bootstrapper v2 config with the dynamic Madara RPC URL
                let madara_endpoint = madara.rpc_endpoint().to_string();
                BootstrapperV2Service::update_config_file("madara.rpc_url", &madara_endpoint)?;
                println!("✅ Updated bootstrapper v2 config with madara.rpc_url: {}", madara_endpoint);
            }

            self.bootstrap_madara().await?;

            // Get the block number for syncing
            if let Some(madara) = &services.madara_service {
                self.bootstrapped_madara_block_number = madara
                    .get_latest_block_number()
                    .await
                    .map_err(|err| SetupError::Madara(MadaraError::RpcError(err)))?;
            }

            println!("✅ L2 Setup completed");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("L2 setup timed out".to_string()))?
    }

    async fn setup_full_node_syncing(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting Pathfinder syncing till # Block {:?}", self.bootstrapped_madara_block_number);

        let duration = self.config.get_timeouts().complete_full_node_syncing;

        timeout(duration, async {
            self.start_pathfinder(services).await?;

            if let Some(pathfinder) = &services.pathfinder_service {
                if let Some(sync_block) = self.bootstrapped_madara_block_number {
                    pathfinder.wait_for_block_synced(sync_block).await?;
                }
            }

            // Stop Madara block production after pathfinder syncs
            if let Some(madara) = &services.madara_service {
                madara.stop_block_production_madara().await?;
                println!("🛑 Madara block production stopped after Pathfinder sync");
            }

            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Pathfinder syncing timed out".to_string()))?
    }

    // async fn setup_mock_prover(&self, services: &mut RunningServices) -> Result<(), SetupError> {
    //     self.start_mock_prover(services).await
    // }

    async fn setup_orchestration(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        println!("🎯 Starting Orchestration...");

        let duration = self.config.get_timeouts().complete_orchestration;

        timeout(duration, async {
            self.start_orchestrator(services).await?;

            self.wait_for_orchestrator_sync(services).await?;
            self.dump_databases(services).await?;

            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Orchestration setup timed out".to_string()))?
    }

    async fn wait_for_orchestrator_sync(&self, services: &RunningServices) -> Result<(), SetupError> {
        let delay = Duration::from_secs(120); // Check every 2 mins
        let timeout = Duration::from_secs(1800); // For 30 mins

        let operation = || async move {
            println!("⏳ Checking orchestrator state update...");

            if let Some(orchestrator) = &services.orchestrator_service {
                let mut is_synced = false;
                if let Some(sync_block) = self.bootstrapped_madara_block_number {
                    is_synced = orchestrator.check_state_update(sync_block).await.map_err(SetupError::Orchestrator)?;
                }

                if is_synced {
                    return Ok(());
                }
            }

            println!("😩 Retrying orchestrator state update check...");
            Err(SetupError::Orchestrator(OrchestratorError::NotSynced))
        };

        retry_with_timeout(delay, timeout, operation)
            .await
            .map_err(|_| SetupError::Timeout("Orchestrator sync timed out".to_string()))
    }

    async fn dump_databases(&self, services: &RunningServices) -> Result<(), SetupError> {
        println!("🏗️ Dumping databases...");

        let duration = self.config.get_timeouts().setup_mongodb_infrastructure_services;

        timeout(duration, async {
            if let Some(mongo) = &services.mongo_service {
                println!("Dumping MongoDB database...");
                mongo.dump_db(DATA_DIR, ORCHESTRATOR_DATABASE_NAME).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Mongodb Infrastructure setup timed out".to_string()))?
    }

    async fn restore_mongodb_database(&self, services: &RunningServices) -> Result<(), SetupError> {
        println!("🏗️ Setting up mongodb infrastructure...");

        let duration = self.config.get_timeouts().setup_mongodb_infrastructure_services;

        timeout(duration, async {
            if let Some(ref mongo) = services.mongo_service {
                mongo.restore_db(DATA_DIR, ORCHESTRATOR_DATABASE_NAME).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Mongodb Infrastructure setup timed out".to_string()))?
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

    // async fn start_mock_prover(&self, services: &mut RunningServices) -> Result<(), SetupError> {
    //     println!("🔔 Starting Mock Prover Service");

    //     let duration = self.config.get_timeouts().start_mock_prover;

    //     timeout(duration, async {
    //         let mock_prover_config = self.config.get_mock_prover_config().clone();
    //         let mock_prover_service = MockProverService::start(mock_prover_config).await?;
    //         services.mock_prover_service = Some(mock_prover_service);
    //         println!("✅ Mock Prover Service started");
    //         Ok(())
    //     })
    //     .await
    //     .map_err(|_| SetupError::Timeout("Mock Prover startup timed out".to_string()))?
    // }

    // Deployment and bootstrap methods
    async fn deploy_mock_verifier(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Deploying mock verifier...");

        let mock_verifier_config = self.config.get_mock_verifier_deployer_config().clone();
        let address = MockVerifierDeployerService::run(mock_verifier_config).await?;

        println!("🥳 Mock verifier deployed at address {}", address);
        // Update v2 config file with verifier address (nested path)
        BootstrapperV2Service::update_config_file("base_layer.core_contract_init_data.verifier", address.as_str())?;

        Ok(())
    }

    async fn bootstrap_base(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Bootstrapping base layer...");

        let bootstrapper_config = self.config.get_bootstrapper_setup_base_config().clone();
        let status = BootstrapperV2Service::run(bootstrapper_config).await?;

        println!("🥳 Base layer bootstrapper finished with {}", status);

        // Update madara config with the deployed core contract address
        self.update_madara_config_with_core_contract()?;

        Ok(())
    }

    fn update_madara_config_with_core_contract(&self) -> Result<(), SetupError> {
        use crate::services::constants::{BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT, DATA_DIR, MADARA_CONFIG};
        use crate::services::helpers::get_file_path;

        // Read the deployed addresses from base_addresses.json
        let addresses_path = get_file_path(&format!("{}/{}", DATA_DIR, BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT));
        let addresses_content = std::fs::read_to_string(&addresses_path)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigReadWriteError(e)))?;
        let addresses: serde_json::Value = serde_json::from_str(&addresses_content)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigParseError(e)))?;

        // Extract the core contract address from the deployed addresses
        let core_contract_address = addresses["addresses"]["coreContract"]
            .as_str()
            .ok_or_else(|| SetupError::BootstrapperV2(
                BootstrapperV2Error::InvalidConfig("Core contract address not found in base_addresses.json".to_string())
            ))?;

        // Read the madara config file
        let madara_config_path = get_file_path(MADARA_CONFIG);
        let madara_config_content = std::fs::read_to_string(&madara_config_path)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigReadWriteError(e)))?;

        // Replace the eth_core_contract_address line
        let updated_content = madara_config_content
            .lines()
            .map(|line| {
                if line.starts_with("eth_core_contract_address:") {
                    format!("eth_core_contract_address: \"{}\"", core_contract_address)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Write the updated config back
        std::fs::write(&madara_config_path, updated_content)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigReadWriteError(e)))?;

        println!("✅ Updated madara config with eth_core_contract_address: {}", core_contract_address);

        Ok(())
    }

    async fn bootstrap_madara(&self) -> Result<(), SetupError> {
        println!("🧑‍💻 Bootstrapping Madara...");

        let bootstrapper_config = self.config.get_bootstrapper_setup_madara_config().clone();
        let status = BootstrapperV2Service::run(bootstrapper_config).await?;

        println!("🥳 Madara bootstrapper finished with {}", status);
        Ok(())
    }

    async fn start_orchestrator(&self, services: &mut RunningServices) -> Result<(), SetupError> {
        // Read the core contract address from bootstrapper output and add it to orchestrator config
        let orchestrator_config = self.get_orchestrator_config_with_core_contract()?;
        let orchestrator_service = OrchestratorService::run(orchestrator_config).await?;
        services.orchestrator_service = Some(orchestrator_service);
        Ok(())
    }

    /// Get orchestrator config with the core contract address from bootstrapper output
    fn get_orchestrator_config_with_core_contract(&self) -> Result<OrchestratorConfig, SetupError> {
        use crate::services::constants::{BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT, DATA_DIR};
        use crate::services::helpers::get_file_path;

        // Read the deployed addresses from base_addresses.json
        let addresses_path = get_file_path(&format!("{}/{}", DATA_DIR, BOOTSTRAPPER_V2_BASE_ADDRESSES_OUTPUT));
        let addresses_content = std::fs::read_to_string(&addresses_path)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigReadWriteError(e)))?;
        let addresses: serde_json::Value = serde_json::from_str(&addresses_content)
            .map_err(|e| SetupError::BootstrapperV2(BootstrapperV2Error::ConfigParseError(e)))?;

        // Extract the core contract address
        let core_contract_address = addresses["addresses"]["coreContract"]
            .as_str()
            .ok_or_else(|| SetupError::BootstrapperV2(
                BootstrapperV2Error::InvalidConfig("Core contract address not found in base_addresses.json".to_string())
            ))?;

        println!("✅ Using core contract address for orchestrator: {}", core_contract_address);

        // Clone the base config and add the core contract address env var
        let orchestrator_config = self.config.get_orchestrator_run_config().clone()
            .builder()
            .env_var("MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS", core_contract_address)
            .build();

        Ok(orchestrator_config)
    }
}
