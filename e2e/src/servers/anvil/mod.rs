// =============================================================================
// ANVIL SERVICE - Spawns a new Anvil service with the given configuration
// =============================================================================


pub mod config;
// Re-export common utilities
pub use config::*;

use crate::servers::server::{Server, ServerConfig};
use tokio::process::Command;

// Anvil service that uses the generic Server
pub struct AnvilService {
    server: Server,
    config: AnvilConfig,
}

impl AnvilService {
    /// Start a new Anvil service with the given configuration
    pub async fn start(config: AnvilConfig) -> Result<Self, AnvilError> {
        // Validate that anvil is present in the system
        if !Self::check_anvil_installed().await {
            return Err(AnvilError::NotInstalled);
        }

        // Build the anvil command
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            port: config.port(),
            host: config.host().to_string(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(AnvilError::Server)?;

        Ok(Self { server, config })
    }


    /// Check if Anvil is installed on the system
    async fn check_anvil_installed() -> bool {
        Command::new("anvil")
            .arg("--version")
            .output()
            .await
            .is_ok()
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub fn dependencies(&self) -> Option<Vec<String>> {
        Some(vec![])
    }
}
