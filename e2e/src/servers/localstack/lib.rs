// =============================================================================
// LOCALSTACK SERVICE - Using Docker and generic Server
// =============================================================================

use super::util::{LocalstackConfig, LocalstackError};
use crate::servers::server::{Server, ServerConfig};
use tokio::process::Command;
use crate::servers::docker::{DockerError, DockerServer};

pub struct LocalstackService {
    server: Server,
    config: LocalstackConfig,
}

impl LocalstackService {
    /// Start a new Localstack service
    /// Will panic if Localstack is already running as per your requirement
    pub async fn start(config: LocalstackConfig) -> Result<Self, LocalstackError> {
        // Validate Docker is running
        if !DockerServer::is_docker_running() {
            return Err(LocalstackError::Docker(DockerError::NotRunning));
        }

        // Check if container is already running - PANIC as requested
        if DockerServer::is_container_running(&config.container_name)? {
            panic!(
                "Localstack container '{}' is already running on port {}. Please stop it first.",
                config.container_name, config.port
            );
        }

        // Check if port is in use
        if DockerServer::is_port_in_use(config.port) {
            return Err(LocalstackError::PortInUse(config.port));
        }

        // Clean up any existing stopped container with the same name
        if DockerServer::does_container_exist(&config.container_name)? {
            DockerServer::remove_container(&config.container_name)?;
        }

        // Build the docker command
        let command = Self::build_docker_command(&config);

        // Create server config
        let server_config = ServerConfig {
            port: config.port,
            connection_attempts: 60, // Localstack takes longer to start
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(|e| LocalstackError::Docker(DockerError::Server(e)))?;

        Ok(Self { server, config })
    }

    /// Build the Docker command for Localstack
    fn build_docker_command(config: &LocalstackConfig) -> Command {
        let mut command = Command::new("docker");
        command.arg("run");
        command.arg("--rm"); // Remove container when it stops
        command.arg("--name").arg(&config.container_name);
        command.arg("-p").arg(format!("{}:{}", config.port, config.port));

        // Add environment variables
        for (key, value) in &config.environment_vars {
            command.arg("-e").arg(format!("{}={}", key, value));
        }

        // Add AWS prefix if specified
        if let Some(prefix) = &config.aws_prefix {
            command.arg("-e").arg(format!("AWS_PREFIX={}", prefix));
        }

        command.arg(&config.image);

        command
    }

    /// Validate if AWS resources with the given prefix are available
    /// This helps determine if the scenario setup is ready
    pub async fn validate_resources(&self, aws_prefix: &str) -> Result<bool, LocalstackError> {
        // This is a basic implementation - you might want to extend this
        // to check specific resources like S3 buckets, DynamoDB tables, etc.

        // Example: Check if we can connect to Localstack's health endpoint
        let health_url = format!("http://{}:{}/health", self.server.host(), self.server.port());

        match reqwest::get(&health_url).await {
            Ok(response) => {
                if response.status().is_success() {
                    // You can add more specific validation here
                    // For example, check if specific AWS resources exist
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
            Err(_) => Ok(false),
        }
    }

    pub fn server(&self) -> &Server {
        &self.server
    }
}
