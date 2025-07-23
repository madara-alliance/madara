// =============================================================================
// SETUP MODULE - MAIN ENTRY POINT
// =============================================================================

pub mod config;
pub mod dependency_validation;
pub mod database_management;
pub mod lifecycle_management;
pub mod service_management;

pub use service_management::*;
pub use dependency_validation::*;
use std::sync::Arc;
use crate::setup::database_management::DatabaseManager;
use crate::setup::lifecycle_management::ServiceLifecycleManager;

// =============================================================================
// MAIN SETUP FACADE
// =============================================================================

pub struct ChainSetup {
    config: Arc<SetupConfig>,
    validator: DependencyValidator,
    database_manager: DatabaseManager,
    service_manager: ServiceManager,
    lifecycle_manager: ServiceLifecycleManager,
}

impl ChainSetup {
    pub fn new(config: SetupConfig) -> Result<Self, SetupError> {
        let config = Arc::new(config);

        Ok(Self {
            service_manager: ServiceManager::new(config.clone()),
            database_manager: DatabaseManager::new(),
            lifecycle_manager: ServiceLifecycleManager::new(),
            validator: DependencyValidator::new(
                config.layer.clone(),
                config.get_timeouts().validate_dependencies
            ),
            config
        })
    }

    pub async fn setup(&mut self) -> Result<(), SetupError> {
        println!("🚀 Starting Chain Setup for {:?} layer...", self.config.layer);

        let db_status = self.database_manager.check_existing_state().await?;

        match db_status {
            DBState::ReadyToUse => {
                println!("✅ Chain state exists, starting servers...");
                self.start_existing_chain().await?;
            },
            DBState::Locked => {
                println!("⚠️ Chain state is locked, waiting for unlock...");
                self.wait_for_unlock_and_retry().await?;
            },
            DBState::NotReady => {
                println!("❌ Chain state does not exist, setting up new chain...");
                let test_config = self.config.to_owned();
                let setup_config = SetupConfigBuilder::new(None).build_l2_config()?;
                self.config = Arc::new(setup_config);
                self.service_manager = ServiceManager::new(self.config.clone());
                self.setup_new_chain().await?;
                self.config = test_config;
                self.service_manager = ServiceManager::new(self.config.clone());
                self.start_existing_chain().await?;
            },
            DBState::Error => {
                return Err(SetupError::OtherError("Invalid DB status".to_string()));
            }
        }

        Ok(())
    }

    async fn setup_new_chain(&mut self) -> Result<(), SetupError> {
        // Validate dependencies
        self.validator.validate_all().await?;

        // Setup new chain using service manager
        self.service_manager.setup_new_chain().await?;

        // Mark database as ready
        self.database_manager.mark_as_ready().await?;

        Ok(())
    }

    async fn start_existing_chain(&mut self) -> Result<(), SetupError> {
        // Copy databases for test isolation
        // TODO: remove the hardcoding
        self.database_manager.copy_for_test("e2e_setup_test").await?;

        // Start services
        let services = self.service_manager.start_runtime_services().await?;

        // Register services with lifecycle manager
        self.lifecycle_manager.register_services(services);

        Ok(())
    }

    async fn wait_for_unlock_and_retry(&mut self) -> Result<(), SetupError> {
        // TODO: Implement with proper timeout and retry logic
        unimplemented!("Implement timeout and re-call setup");
    }

    pub async fn shutdown(&mut self) -> Result<(), SetupError> {
        self.lifecycle_manager.shutdown_all().await
    }
}

impl Drop for ChainSetup {
    fn drop(&mut self) {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let mut lifecycle = std::mem::take(&mut self.lifecycle_manager);
            handle.spawn(async move {
                let _ = lifecycle.shutdown_all().await;
            });
        } else if let Ok(rt) = tokio::runtime::Runtime::new() {
            let _ = rt.block_on(self.lifecycle_manager.shutdown_all());
        }
    }
}

// Re-export the main facade for easy usage
pub use ChainSetup as Setup;
