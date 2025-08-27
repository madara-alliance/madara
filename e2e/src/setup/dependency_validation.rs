// =============================================================================
// DEPENDENCY VALIDATION
// =============================================================================

use tokio::process::Command;
use tokio::task::JoinSet;
// Import all the services we've created
pub use super::config::*;
use crate::services::constants::*;
use crate::services::docker::DockerServer;
use crate::services::orchestrator::Layer;
use tokio::time::timeout;
use tokio::time::Duration;

pub struct DependencyValidator {
    layer: Layer,
    validate_deps_timeout: Duration,
}

impl DependencyValidator {
    pub fn new(layer: Layer, validate_deps_timeout: Duration) -> Self {
        Self { layer, validate_deps_timeout }
    }

    pub async fn validate_all(&self) -> Result<(), SetupError> {
        let duration = self.validate_deps_timeout;

        timeout(duration, async {
            println!("🔍 Validating dependencies...");

            self.validate_system_tools().await?;
            self.validate_docker_images().await?;

            println!("✅ All dependencies validated");
            Ok(())
        })
        .await
        .map_err(|_| SetupError::Timeout("Dependency validation timed out".to_string()))?
    }

    async fn validate_system_tools(&self) -> Result<(), SetupError> {
        let mut join_set = JoinSet::new();

        // Only validate L2 tools if needed
        if self.layer == Layer::L2 {
            join_set.spawn(Self::check_tool("anvil", "Anvil"));
            join_set.spawn(Self::check_tool("forge", "Forge"));
        }

        join_set.spawn(async {
            if !DockerServer::is_docker_running().await {
                return Err(SetupError::DependencyFailed("Docker not running".to_string()));
            }
            println!("✅ Docker is running");
            Ok(())
        });

        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        Ok(())
    }

    async fn check_tool(command: &str, display_name: &str) -> Result<(), SetupError> {
        Command::new(command)
            .arg("--version")
            .output()
            .await
            .map_err(|_| SetupError::DependencyFailed(format!("{} not found", display_name)))?;

        println!("✅ {} is available", display_name);
        Ok(())
    }

    async fn validate_docker_images(&self) -> Result<(), SetupError> {
        println!("📦 Pulling required Docker images...");

        let images = vec![("mongo", MONGODB_IMAGE), ("localstack/localstack", LOCALSTACK_IMAGE)];

        let mut join_set = JoinSet::new();

        for (display_name, image_name) in images {
            join_set.spawn(Self::pull_image(display_name, image_name));
        }

        while let Some(result) = join_set.join_next().await {
            result.map_err(|e| SetupError::DependencyFailed(e.to_string()))??;
        }

        Ok(())
    }

    async fn pull_image(display_name: &str, image_name: &str) -> Result<(), SetupError> {
        println!("📦 Pulling {}...", display_name);

        let output = Command::new("docker").args(["pull", image_name]).output().await.map_err(|e| {
            SetupError::DependencyFailed(format!("Failed to execute docker pull {}: {}", display_name, e))
        })?;

        if output.status.success() {
            println!("✅ Successfully pulled {}", display_name);
            Ok(())
        } else {
            let error_msg = String::from_utf8_lossy(&output.stderr);
            Err(SetupError::DependencyFailed(format!("Failed to pull {}: {}", display_name, error_msg)))
        }
    }
}
