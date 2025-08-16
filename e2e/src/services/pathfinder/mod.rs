// =============================================================================
// PATHFINDER SERVICE - Using generic Server
// =============================================================================

pub mod config;

use crate::services::helpers::NodeRpcMethods;
use crate::services::server::{Server, ServerConfig};
pub use config::*;
use reqwest::Url;

pub struct PathfinderService {
    server: Server,
    config: PathfinderConfig,
}

impl PathfinderService {
    /// Start a new Pathfinder service
    /// Will panic if Pathfinder is already running as per pattern
    pub async fn start(config: PathfinderConfig) -> Result<Self, PathfinderError> {
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            rpc_port: Some(config.port()),
            service_name: "Pathfinder".to_string(),
            connection_attempts: 60, // Pathfinder takes time to sync
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(PathfinderError::Server)?;

        Ok(Self { server, config })
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

    /// Get the RPC endpoint URL
    pub fn endpoint(&self) -> Url {
        self.server().endpoint().expect("Failed to get endpoint")
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
    pub fn ethereum_url(&self) -> &Url {
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
        self.server.stop().map_err(PathfinderError::Server)?;
        Ok(())
    }

    pub async fn wait_for_block_synced(&self, block_number: u64) -> Result<(), PathfinderError> {
        self.wait_for_block(block_number).await.map_err(|err| PathfinderError::RpcError(err))
    }
}

impl NodeRpcMethods for PathfinderService {
    fn get_endpoint(&self) -> Url {
        self.endpoint().clone()
    }
}
