// =============================================================================
// MADARA SERVICE - Starknet Sequencer using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::services::server::{Server, ServerConfig};
use reqwest::Url;
use std::path::PathBuf;
use tokio::time::Duration;
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

        let poll_interval = Duration::from_millis(500); // Configurable interval
        let mut retry_count = 0;
        const MAX_RETRIES: u32 = 1200; // 10 minutes with 500ms intervals

        loop {
            match self.get_latest_block_number().await {
                Ok(Some(latest)) => {
                    if latest >= block_number {
                        println!("🔔 Madara block {} is mined (latest: {})", block_number, latest);
                        return Ok(());
                    }
                }
                Ok(None) => {
                    // No blocks mined yet, continue waiting
                }
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= MAX_RETRIES {
                        return Err(MadaraError::TimeoutWaitingForBlock(block_number, MAX_RETRIES, e.to_string()));
                    }

                    // Log error but continue retrying
                    if retry_count % 20 == 0 {
                        // Log every ~10 seconds
                        println!("⚠️  Error fetching block number (retry {}/{}): {}",
                                retry_count, MAX_RETRIES, e.to_string());
                    }
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

}

impl NodeRpcMethods for MadaraService {
    fn get_endpoint(&self) -> Url {
        self.endpoint().clone()
    }
}
