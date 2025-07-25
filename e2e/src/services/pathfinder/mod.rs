// =============================================================================
// PATHFINDER SERVICE - Using generic Server
// =============================================================================

pub mod config;

use crate::services::helpers::NodeRpcMethods;
use crate::services::server::{Server, ServerConfig};
pub use config::*;
use reqwest::Url;
use tokio::time::Duration;

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
            service_name: format!("Pathfinder"),
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
        self.server.stop().map_err(|err| PathfinderError::Server(err))
    }

    pub async fn wait_for_block_synced(&self, block_number: u64) -> Result<(), PathfinderError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        println!("⏳ Waiting for Pathfinder block {} to be synced", block_number);

        while self.get_latest_block_number().await.map_err(|err| PathfinderError::RpcError(err))? < 0 {
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
