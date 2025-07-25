// =============================================================================
// ANVIL SERVICE - Spawns a new Anvil service with the given configuration
// =============================================================================

pub mod config;

// Re-export common utilities
pub use config::*;
use url::Url;

use crate::services::server::{Server, ServerConfig};

// Anvil service that uses the generic Server
pub struct AnvilService {
    server: Server,
    config: AnvilConfig,
}

impl AnvilService {
    /// Start a new Anvil service with the given configuration
    pub async fn start(config: AnvilConfig) -> Result<Self, AnvilError> {

        // Build the anvil command
        let command = config.to_command();

        // Create server config using the immutable config getters
        let server_config = ServerConfig {
            rpc_port: Some(config.port()),
            logs: config.logs(),
            service_name: "Anvil".to_string(),
            ..Default::default()
        };

        // Start the server using the generic Server::start_process
        let server = Server::start_process(command, server_config)
            .await
            .map_err(AnvilError::Server)?;

        Ok(Self { server, config })
    }

    pub fn server(&self) -> &Server {
        &self.server
    }

    pub async fn stop(&mut self) -> Result<(), AnvilError> {
        println!("☠️ Stopping Anvil");
        self.server.stop().await.map_err(|err| AnvilError::Server(err))
    }

    pub fn config(&self) -> &AnvilConfig {
        &self.config
    }

    pub fn endpoint(&self) -> Url {
        self.server().endpoint().expect("Anvil should have an endpoint")
    }
}
