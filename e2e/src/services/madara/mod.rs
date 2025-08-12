// =============================================================================
// MADARA SERVICE - Starknet Sequencer using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use reqwest::Url;
use std::path::PathBuf;

use crate::services::constants::*;
use crate::services::helpers::NodeRpcMethods;

pub struct MadaraService {
    server: Server,
    config: MadaraConfig,
}

impl MadaraService {
    /// Start a new Madara service
    pub async fn start(config: MadaraConfig) -> Result<Self, MadaraError> {
        // TODO: Validation should move to madara config

        // Build the command using the immutable config
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            rpc_port: Some(config.rpc_port()),
            service_name: format!("Madara-{}", config.mode().to_string()),
            connection_attempts: 60, // Madara might take time to start
            connection_delay_ms: 2000,
            logs: config.logs(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config).await.map_err(MadaraError::Server)?;

        Ok(Self { server, config })
    }

    // TODO: deps should be an enum
    /// Get the dependencies required by Madara
    pub fn dependencies(&self) -> Vec<String> {
        vec!["anvil".to_string()] // L1 endpoint dependency
    }

    /// Get the RPC endpoint URL
    pub fn rpc_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", DEFAULT_SERVICE_HOST, self.config().rpc_port())).unwrap()
    }


    /// Get the main endpoint URL (alias for rpc_endpoint)
    pub fn endpoint(&self) -> Url {
        self.rpc_endpoint()
    }

    /// Get the RPC port number
    pub fn port(&self) -> u16 {
        self.config().rpc_port()
    }

    /// Get the RPC admin port number
    pub fn rpc_admin_port(&self) -> u16 {
        self.config().rpc_admin_port()
    }

    /// Get the Gateway port number
    pub fn gateway_port(&self) -> u16 {
        self.config.gateway_port()
    }

    /// Get the chain name
    pub fn name(&self) -> &str {
        self.config.name()
    }

    /// Get the base path
    pub fn database_path(&self) -> &PathBuf {
        self.config.database_path()
    }

    /// Get the underlying server
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// Get the current configuration
    pub fn config(&self) -> &MadaraConfig {
        &self.config
    }

    pub fn stop(&mut self) -> Result<(), MadaraError> {
        println!("☠️ Stopping Madara");
        self.server.stop().map_err(|err| MadaraError::Server(err))
    }

    /// Get the process ID
    pub fn pid(&self) -> Option<u32> {
        self.server.pid()
    }

    /// Check if the service is running
    pub fn is_running(&mut self) -> Result<bool, MadaraError> {
        Ok(self.server.has_exited()?.is_none())
    }

    pub async fn wait_for_block_mined(&self, block_number: u64) -> Result<(), MadaraError> {
        println!("⏳ Waiting for Madara block {} to be mined", block_number);

        while self.get_latest_block_number().await.map_err(|err| MadaraError::RpcError(err))? < 0 {
            println!("⏳ Checking Madara block status...");
            tokio::time::sleep(MADARA_WAITING_DURATION.to_owned()).await;
        }
        println!("🔔 Madara block {} is mined", block_number);

        Ok(())
    }
}

impl NodeRpcMethods for MadaraService {
    fn get_endpoint(&self) -> Url {
        self.endpoint().clone()
    }
}
