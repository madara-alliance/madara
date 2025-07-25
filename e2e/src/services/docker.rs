// =============================================================================
// DOCKER SERVER HELPER - For Docker-based services
// =============================================================================

use crate::services::constants::DEFAULT_SERVICE_HOST;
use crate::services::server::ServerError;
use tokio::process::Command;
use crate::services::constants::DEFAULT_SERVICE_HOST;

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Docker is not running or not installed")]
    NotRunning,
    #[error("Container already exists: {0}")]
    ContainerAlreadyExists(String),
    #[error("Failed to check container status: {0}")]
    ContainerStatusFailed(std::io::Error),
    #[error("Server error: {0}")]
    Server(#[from] ServerError),
    #[error("Exec error: {0}")]
    Exec(String),
}

pub struct DockerServer;

impl DockerServer {
    /// Check if Docker is running
    pub async fn is_docker_running() -> bool {
        let result = Command::new("docker").arg("info").output().await;

        match result {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }

    /// Check if a container with the given name is already running
    pub async fn is_container_running(container_name: &str) -> Result<bool, DockerError> {
        let output = Command::new("docker")
            .args(["ps", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .await
            .map_err(DockerError::ContainerStatusFailed)?;

        Ok(!output.stdout.is_empty())
    }

    /// Check if a container with the given name exists (running or stopped)
    pub async fn does_container_exist(container_name: &str) -> Result<bool, DockerError> {
        let output = Command::new("docker")
            .args(["ps", "-a", "-q", "-f", &format!("name={}", container_name)])
            .output()
            .await
            .map_err(DockerError::ContainerStatusFailed)?;

        Ok(!output.stdout.is_empty())
    }

    /// Remove a container (force remove if running)
    pub async fn remove_container(container_name: &str) -> Result<(), DockerError> {
        let _output = Command::new("docker")
            .args(["rm", "-f", container_name])
            .output()
            .await
            .map_err(DockerError::ContainerStatusFailed)?;

        Ok(())
    }

    /// Check if a port is in use
    pub fn is_port_in_use(port: u16) -> bool {
        std::net::TcpListener::bind(format!("{}:{}", DEFAULT_SERVICE_HOST, port)).is_err()
    }
}
