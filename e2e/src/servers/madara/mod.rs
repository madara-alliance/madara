// =============================================================================
// MADARA SERVICE - Starknet Sequencer using generic Server
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;

use crate::servers::server::{Server, ServerConfig};
use reqwest::Url;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

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

        println!("Starting Madara service with command: {:?}", command);

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            port: config.rpc_port(),
            host: "127.0.0.1".to_string(), // Default host for Madara
            connection_attempts: 60, // Madara might take time to start
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(MadaraError::Server)?;

        Ok(Self { server, config })
    }

    // TODO: deps should be an enum
    /// Get the dependencies required by Madara
    pub fn dependencies(&self) -> Vec<String> {
        vec!["anvil".to_string()] // L1 endpoint dependency
    }

    // TODO: ideally validating deps should be done inside setup coz it has the deps listed within itself as Option
    /// Validate that all required dependencies are available
    pub fn validate_dependencies(&self) -> Result<(), MadaraError> {
        //  need to move to setup
        Ok(())
    }

    /// Get the RPC endpoint URL
    pub fn rpc_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", self.server.host(), self.server.port())).unwrap()
    }

    /// Get the Gateway endpoint URL
    pub fn gateway_endpoint(&self) -> Url {
        Url::parse(&format!("http://{}:{}", self.server.host(), self.config.gateway_port())).unwrap()
    }

    /// Get the Feeder Gateway endpoint URL
    pub fn feeder_gateway_endpoint(&self) -> Url {
        Url::parse(&format!(
            "http://{}:{}/feeder_gateway",
            self.server.host(),
            self.config.gateway_port()
        ))
        .unwrap()
    }

    /// Get the main endpoint URL (alias for rpc_endpoint)
    pub fn endpoint(&self) -> Url {
        self.rpc_endpoint()
    }

    /// Get the RPC port number
    pub fn port(&self) -> u16 {
        self.server.port()
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

    /// Get the process ID
    pub fn pid(&self) -> Option<u32> {
        self.server.pid()
    }

    /// Check if the process has exited
    pub fn has_exited(&mut self) -> Option<ExitStatus> {
        self.server.has_exited()
    }

    /// Check if the service is running
    pub fn is_running(&mut self) -> bool {
        self.server.is_running()
    }

    /// Stop the Madara service
    pub fn stop(&mut self) -> Result<(), MadaraError> {
        self.server.stop().map_err(MadaraError::Server)
    }

    /// Restart the Madara service (useful after bootstrapper setup)
    pub async fn restart(&mut self) -> Result<(), MadaraError> {
        println!("🔄 Restarting Madara service...");

        // Stop current instance
        self.stop()?;

        // Wait a moment for clean shutdown
        tokio::time::sleep(Duration::from_secs(10)).await;

        // Build new command
        let command = self.config.to_command();

        // Create server config
        let server_config = ServerConfig {
            port: self.config.rpc_port(),
            host: "127.0.0.1".to_string(),
            connection_attempts: 60,
            connection_delay_ms: 2000,
            ..Default::default()
        };

        // Start new instance
        self.server = Server::start_process(command, server_config)
            .await
            .map_err(MadaraError::Server)?;

        println!("✅ Madara service restarted");
        Ok(())
    }

    /// Create database directory if it doesn't exist
    pub async fn ensure_database_directory(&self) -> Result<(), MadaraError> {
        if !self.config.database_path().exists() {
            tokio::fs::create_dir_all(self.config.database_path()).await?;
            println!("📁 Created database directory: {}", self.config.database_path().display());
        }
        Ok(())
    }

    /// Check if database has been initialized
    pub fn is_database_initialized(&self) -> bool {
        self.config.database_path().exists() && self.config.database_path().join("db").exists()
    }

    /// Get database size in bytes
    pub async fn get_database_size(&self) -> Result<u64, MadaraError> {
        if !self.config.database_path().exists() {
            return Ok(0);
        }

        let mut size = 0u64;
        let mut entries = tokio::fs::read_dir(self.config.database_path()).await?;

        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            if metadata.is_file() {
                size += metadata.len();
            }
        }

        Ok(size)
    }
}
