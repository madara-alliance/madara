// =============================================================================
// SERVICE LIFECYCLE MANAGEMENT MODULE
// =============================================================================

use crate::setup::service_management::RunningServices;
use crate::setup::SetupError;
use tokio::time::{sleep,Duration};

const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ServiceLifecycleManager {
    services: Option<RunningServices>,
}

impl ServiceLifecycleManager {
    pub fn new() -> Self {
        Self {
            services: None,
        }
    }

    pub fn register_services(&mut self, services: RunningServices) {
        self.services = Some(services);
    }

    pub async fn shutdown_all(&mut self) -> Result<(), SetupError> {
        if let Some(mut services) = self.services.take() {
            println!("🛑 Shutting down all services...");
            // Shutdown in reverse dependency order

            if let Some(mut orchestrator) = services.orchestrator_service.take() {
                let _ = orchestrator.stop()?;
                println!("🛑 Orchestrator stopped");
            }

            if let Some(mut mock_prover) = services.mock_prover_service.take() {
                let _ = mock_prover.stop()?;
                println!("🛑 Mock Prover stopped");
            }

            if let Some(mut pathfinder) = services.pathfinder_service.take() {
                let _ = pathfinder.stop()?;
                println!("🛑 Pathfinder stopped");
            }

            if let Some(mut madara) = services.madara_service.take() {
                let _ = madara.stop()?;
                println!("🛑 Madara stopped");
            }

            if let Some(mut anvil) = services.anvil_service.take() {
                let _ = anvil.stop()?;
                println!("🛑 Anvil stopped");
            }

            if let Some(mut mongo) = services.mongo_service.take() {
                let _ = mongo.stop()?;
                println!("🛑 MongoDB stopped");
            }

            if let Some(mut localstack) = services.localstack_service.take() {
                let _ = localstack.stop()?;
                println!("🛑 Localstack stopped");
            }

            println!("✅ All services stopped gracefully");
        }

        // Docker takes a while to close the containers
        sleep(SHUTDOWN_TIMEOUT).await;

        Ok(())
    }
}

impl Default for ServiceLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}
