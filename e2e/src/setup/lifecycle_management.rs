// =============================================================================
// SERVICE LIFECYCLE MANAGEMENT MODULE
// =============================================================================

use crate::services::madara::MadaraService;
use crate::setup::service_management::RunningServices;
use crate::setup::SetupError;
use tokio::time::{sleep, Duration};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ServiceLifecycleManager {
    pub services: Option<RunningServices>,
}

impl ServiceLifecycleManager {
    pub fn new() -> Self {
        Self { services: None }
    }

    pub fn register_services(&mut self, services: RunningServices) {
        self.services = Some(services);
    }

    pub async fn shutdown_all(&mut self) -> Result<(), SetupError> {
        if let Some(mut services) = self.services.take() {
            println!("🛑 Shutting down all services...");
            // Shutdown in reverse dependency order

            if let Some(mut orchestrator) = services.orchestrator_service.take() {
                orchestrator.stop()?;
                println!("🛑 Orchestrator stopped");
            }

            if let Some(mut mock_prover) = services.mock_prover_service.take() {
                mock_prover.stop()?;
                println!("🛑 Mock Prover stopped");
            }

            if let Some(mut pathfinder) = services.pathfinder_service.take() {
                pathfinder.stop()?;
                println!("🛑 Pathfinder stopped");
            }

            if let Some(mut madara) = services.madara_service.take() {
                madara.stop()?;
                println!("🛑 Madara stopped");
            }

            if let Some(mut anvil) = services.anvil_service.take() {
                anvil.stop()?;
                println!("🛑 Anvil stopped");
            }

            if let Some(mut mongo) = services.mongo_service.take() {
                mongo.stop()?;
                println!("🛑 MongoDB stopped");
            }

            if let Some(mut localstack) = services.localstack_service.take() {
                localstack.stop()?;
                println!("🛑 Localstack stopped");
            }

            println!("✅ All services stopped gracefully");
        }

        // Docker takes a while to close the containers
        sleep(SHUTDOWN_TIMEOUT).await;

        Ok(())
    }

    // Getter for madara_service
    // TODO: might be a very good reason to use arc!
    pub fn madara_service(&self) -> &Option<MadaraService> {
        self.services.as_ref().map_or(&None, |s| &s.madara_service)
    }
}

impl Default for ServiceLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}
