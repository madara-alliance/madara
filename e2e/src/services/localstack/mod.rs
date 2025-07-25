// =============================================================================
// LOCALSTACK SERVICE - Using Docker and generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use crate::services::docker::{DockerError, DockerServer};
use url::Url;

pub struct LocalstackService {
    server: Server,
    config: LocalstackConfig,
}

impl LocalstackService {
    /// Start a new Localstack service
    /// Will panic if Localstack is already running as per your requirement
    pub async fn start(config: LocalstackConfig) -> Result<Self, LocalstackError> {
        // Validate Docker is running
        if !DockerServer::is_docker_running().await {
            return Err(LocalstackError::Docker(DockerError::NotRunning));
        }

        // Check if container is already running - PANIC as requested
        if DockerServer::is_container_running(config.container_name()).await? {
            panic!(
                "Localstack container '{}' is already running on port {}. Please stop it first.",
                config.container_name(),
                config.port()
            );
        }

        // Check if port is in use
        if DockerServer::is_port_in_use(config.port()) {
            return Err(LocalstackError::PortInUse(config.port()));
        }

        // Clean up any existing stopped container with the same name
        if DockerServer::does_container_exist(config.container_name()).await? {
            DockerServer::remove_container(config.container_name()).await?;
        }

        // Build the docker command
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            rpc_port: Some(config.port()),
            service_name: "Localstack".to_string(),
            connection_attempts: 60, // Localstack takes longer to start
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(|e| LocalstackError::Docker(DockerError::Server(e)))?;

        Ok(Self { server, config })
    }


    /// Validate if AWS resources with the given prefix are available
    /// This helps determine if the scenario setup is ready
    pub async fn validate_resources(&self, aws_prefix: &str) -> Result<bool, LocalstackError> {
        // This is a basic implementation - you might want to extend this
        // to check specific resources like S3 buckets, DynamoDB tables, etc.

        // Example: Check if we can connect to Localstack's health endpoint
        let health_url = format!("{}/health", self.endpoint());

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

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the configuration used
    pub fn config(&self) -> &LocalstackConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), LocalstackError> {
        println!("☠️ Stopping Localstack");
        self.server.stop().map_err(|err| LocalstackError::Server(err))
    }


    /// Get the endpoint URL for the Localstack server
    pub fn endpoint(&self) -> Url {
        self.server().endpoint().expect("Localstack server endpoint not found!")
    }

}
