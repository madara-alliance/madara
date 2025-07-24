// =============================================================================
// PATHFINDER SERVICE - Using Docker and generic Server
// =============================================================================

pub mod config;

use crate::services::helpers::NodeRpcMethods;
// Re-export common utilities
pub use config::*;
use tokio::time::Duration;
use crate::services::docker::{DockerError, DockerServer};
use crate::services::server::{Server, ServerConfig};
use reqwest::Url;

pub struct PathfinderService {
    server: Server,
    config: PathfinderConfig,
}

impl PathfinderService {
    /// Start a new Pathfinder service
    /// Will panic if Pathfinder is already running as per pattern
    pub async fn start(config: PathfinderConfig) -> Result<Self, PathfinderError> {
        // Validate Docker is running
        if !DockerServer::is_docker_running().await {
            return Err(PathfinderError::Docker(DockerError::NotRunning));
        }

        // Validate required configuration
        Self::validate_config(&config)?;

        // Check if container is already running - PANIC as per pattern
        if DockerServer::is_container_running(config.container_name()).await? {
            panic!(
                "Pathfinder container '{}' is already running on port {}. Please stop it first.",
                config.container_name(),
                config.port()
            );
        }

        // Check if ports are in use
        if DockerServer::is_port_in_use(config.port()) {
            return Err(PathfinderError::PortInUse(config.port()));
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
            service_name: format!("Pathfinder"),
            connection_attempts: 60, // Pathfinder takes time to sync
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(|e| PathfinderError::Docker(DockerError::Server(e)))?;

        Ok(Self { server, config })
    }

    /// Validate the configuration
    fn validate_config(config: &PathfinderConfig) -> Result<(), PathfinderError> {
        if config.ethereum_url().contains("YOUR_API_KEY") {
            return Err(PathfinderError::MissingConfig("ethereum_url must contain a valid API key".to_string()));
        }
        Ok(())
    }


    /// Validate if Pathfinder is ready and responsive
    pub async fn validate_connection(&self) -> Result<bool, PathfinderError> {
        // Try to connect to the RPC endpoint
        let rpc_addr = self.endpoint().to_string();
        match tokio::net::TcpStream::connect(&rpc_addr).await {
            Ok(_) => Ok(true),
            Err(e) => Err(PathfinderError::ConnectionFailed(e.to_string())),
        }
    }

    /// Check if Pathfinder is syncing by making an RPC call
    pub async fn get_sync_status(&self) -> Result<bool, PathfinderError> {
        // In a real implementation, you would make an RPC call to check sync status
        // For now, we'll just check if the connection is available
        self.validate_connection().await
    }

    /// Get the RPC endpoint URL
    pub fn endpoint(&self) -> Url {
        self.server().endpoint()
            .expect("Failed to get endpoint")
    }

    /// Get the network name
    pub fn network(&self) -> &str {
        self.config.network()
    }

    /// Get the chain ID
    pub fn chain_id(&self) -> &str {
        self.config.chain_id()
    }

    /// Get the Ethereum URL
    pub fn ethereum_url(&self) -> &str {
        self.config.ethereum_url()
    }

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the configuration used
    pub fn config(&self) -> &PathfinderConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), PathfinderError> {
        println!("☠️ Stopping Pathfinder");
        self.server.stop().map_err(|err| PathfinderError::Server(err))
    }

    pub async fn wait_for_block_synced(&self, block_number: u64) -> Result<(), PathfinderError> {
        println!("⏳ Waiting for Pathfinder block {} to be synced", block_number);

        while self.get_latest_block_number().await
            .map_err(|err| PathfinderError::RpcError(err))? < 0 {
            println!("⏳ Checking Pathfinder block status...");
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
        println!("🔔 Pathfinder block {} is synced", block_number);

        Ok(())
    }
}


impl NodeRpcMethods for PathfinderService {
    fn get_endpoint(&self) -> Url {
        self.endpoint().clone()
    }
}
